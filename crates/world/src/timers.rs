//! Deterministic countdown timers and active powerup timers. Timers fire
//! in `(due, slot, sequence)` order so a fixed tick replays identically.
//! Powerup shapes follow the `ActivePowerupTimer` contract in
//! `src/contracts/gameplay.ts` (`item`, `label`, `remainingSeconds`).

use qa_core::identity::ActorId;
use qa_core::time::SourceTime;

use crate::registry::ActorRegistry;
use crate::WorldError;

/// One-shot or recurring countdown timer.
#[derive(Debug, Clone, PartialEq)]
pub struct Timer {
    /// Owning actor.
    pub actor: ActorId,
    /// Timer name.
    pub name: String,
    /// Due source time.
    pub due: SourceTime,
    /// Recurrence interval, if recurring.
    pub interval: Option<SourceTime>,
    /// Insertion sequence for deterministic tie-breaks.
    pub sequence: u64,
}

/// Fired timer event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimerFired {
    /// Owning actor.
    pub actor: ActorId,
    /// Timer name.
    pub name: String,
}

/// Active powerup timer (`ActivePowerupTimer` contract).
#[derive(Debug, Clone, PartialEq)]
pub struct PowerupTimer {
    /// Item identifier.
    pub item: String,
    /// Display label.
    pub label: String,
    /// Remaining seconds.
    pub remaining_seconds: f64,
}

impl PowerupTimer {
    /// Advance by `elapsed_seconds`; returns true while still active.
    pub fn tick(&mut self, elapsed_seconds: f64) -> bool {
        self.remaining_seconds -= elapsed_seconds;
        self.remaining_seconds > 0.0
    }
}

/// Deterministic timer table.
#[derive(Debug, Clone, Default)]
pub struct TimerTable {
    timers: Vec<Timer>,
    next_sequence: u64,
    first_unit: Option<SourceTime>,
}

impl TimerTable {
    /// Empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Schedule a timer; due and interval must share the table's unit once
    /// the first timer sets it.
    pub fn schedule(
        &mut self,
        registry: &ActorRegistry,
        actor: &ActorId,
        name: &str,
        due: SourceTime,
        interval: Option<SourceTime>,
    ) -> Result<(), WorldError> {
        if !registry.is_live(actor) {
            return Err(WorldError::StaleActor);
        }
        if matches!(due, SourceTime::Seconds(value) if !value.is_finite()) {
            return Err(WorldError::NonFiniteTime);
        }
        if let Some(interval) = interval {
            if !same_unit(due, interval) {
                return Err(WorldError::TimerUnit);
            }
            if interval_value(interval) < 0.0 {
                return Err(WorldError::NegativeTime);
            }
        }
        if let Some(first) = self.timers.first() {
            if !same_unit(first.due, due) {
                return Err(WorldError::TimerUnit);
            }
        } else if let Some(first) = self.first_unit {
            if !same_unit(first, due) {
                return Err(WorldError::TimerUnit);
            }
        }
        if self.first_unit.is_none() {
            self.first_unit = Some(due);
        }
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        if let Some(slot) = self
            .timers
            .iter_mut()
            .find(|timer| timer.actor == *actor && timer.name == name)
        {
            *slot = Timer {
                actor: actor.clone(),
                name: name.to_string(),
                due,
                interval,
                sequence,
            };
        } else {
            self.timers.push(Timer {
                actor: actor.clone(),
                name: name.to_string(),
                due,
                interval,
                sequence,
            });
        }
        Ok(())
    }

    /// Cancel a named timer.
    pub fn cancel(&mut self, actor: &ActorId, name: &str) {
        self.timers
            .retain(|timer| !(timer.actor == *actor && timer.name == name));
    }

    /// Cancel all timers for an actor.
    pub fn cancel_actor(&mut self, actor: &ActorId) {
        self.timers.retain(|timer| timer.actor != *actor);
    }

    /// Pending timer count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.timers.len()
    }

    /// Whether no timers are pending.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.timers.is_empty()
    }

    /// Fire timers due at or before `now`, in `(due, slot, sequence)`
    /// order. Recurring timers reschedule from `now` plus their interval.
    pub fn advance(&mut self, registry: &ActorRegistry, now: SourceTime) -> Vec<TimerFired> {
        self.timers.retain(|timer| registry.is_live(&timer.actor));
        let mut due: Vec<Timer> = self
            .timers
            .iter()
            .filter(|timer| same_unit(timer.due, now) && interval_value(timer.due) <= interval_value(now))
            .cloned()
            .collect();
        due.sort_by(|left, right| {
            interval_value(left.due)
                .total_cmp(&interval_value(right.due))
                .then(left.actor.slot().cmp(&right.actor.slot()))
                .then(left.sequence.cmp(&right.sequence))
        });
        let mut fired = Vec::new();
        for timer in due {
            self.timers
                .retain(|pending| !(pending.actor == timer.actor && pending.name == timer.name));
            if let Some(interval) = timer.interval {
                let next = add_time(now, interval_value(interval));
                self.timers.push(Timer {
                    due: next,
                    sequence: {
                        let sequence = self.next_sequence;
                        self.next_sequence += 1;
                        sequence
                    },
                    ..timer.clone()
                });
            }
            fired.push(TimerFired {
                actor: timer.actor,
                name: timer.name,
            });
        }
        fired
    }

    /// Checkpoint timers.
    #[must_use]
    pub fn checkpoint(&self) -> Vec<TimerCheckpoint> {
        self.timers
            .iter()
            .map(|timer| TimerCheckpoint {
                slot: timer.actor.slot(),
                generation: timer.actor.generation(),
                name: timer.name.clone(),
                due: timer.due,
                interval: timer.interval,
                sequence: timer.sequence,
            })
            .collect()
    }

    /// Restore timers against a fresh registry.
    pub fn restore(&mut self, registry: &ActorRegistry, saved: &[TimerCheckpoint]) -> Result<(), WorldError> {
        self.timers.clear();
        self.next_sequence = 0;
        for timer in saved {
            let actor = registry
                .live_id(timer.slot, timer.generation)
                .ok_or_else(|| WorldError::BadSave("Timer names a missing actor".to_string()))?;
            self.next_sequence = self.next_sequence.max(timer.sequence + 1);
            self.timers.push(Timer {
                actor,
                name: timer.name.clone(),
                due: timer.due,
                interval: timer.interval,
                sequence: timer.sequence,
            });
        }
        Ok(())
    }
}

/// Saved timer record.
#[derive(Debug, Clone, PartialEq)]
pub struct TimerCheckpoint {
    /// Registry slot.
    pub slot: u32,
    /// Slot generation.
    pub generation: u32,
    /// Timer name.
    pub name: String,
    /// Due source time.
    pub due: SourceTime,
    /// Recurrence interval.
    pub interval: Option<SourceTime>,
    /// Insertion sequence.
    pub sequence: u64,
}

fn same_unit(left: SourceTime, right: SourceTime) -> bool {
    matches!(
        (left, right),
        (SourceTime::Seconds(_), SourceTime::Seconds(_)) | (SourceTime::Milliseconds(_), SourceTime::Milliseconds(_))
    )
}

fn interval_value(time: SourceTime) -> f64 {
    match time {
        SourceTime::Seconds(value) => f64::from(value),
        SourceTime::Milliseconds(value) => f64::from(value),
    }
}

fn add_time(base: SourceTime, delta: f64) -> SourceTime {
    match base {
        SourceTime::Seconds(value) => SourceTime::Seconds(value + delta as f32),
        SourceTime::Milliseconds(value) => SourceTime::Milliseconds(value + delta as i32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::{IdentityOwner, ProviderId};

    fn registry() -> (ActorRegistry, ActorId) {
        let owner = IdentityOwner::create("test").unwrap();
        let mut registry = ActorRegistry::new(owner, 8).unwrap();
        let actor = registry.allocate(ProviderId::new("q1", "game"), "q1:monster").unwrap();
        let id = actor.id().clone();
        std::mem::forget(actor);
        (registry, id)
    }

    #[test]
    fn timers_fire_in_due_slot_sequence_order() {
        let (registry, _) = registry();
        let mut table = TimerTable::new();
        let first = registry.observations()[0].id.clone();
        table
            .schedule(&registry, &first, "b", SourceTime::Milliseconds(200), None)
            .unwrap();
        table
            .schedule(&registry, &first, "a", SourceTime::Milliseconds(100), None)
            .unwrap();
        let fired = table.advance(&registry, SourceTime::Milliseconds(150));
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].name, "a");
        let fired = table.advance(&registry, SourceTime::Milliseconds(200));
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].name, "b");
        assert!(table.is_empty());
    }

    #[test]
    fn recurring_timers_reschedule_from_now() {
        let (registry, _) = registry();
        let mut table = TimerTable::new();
        let first = registry.observations()[0].id.clone();
        table
            .schedule(
                &registry,
                &first,
                "pulse",
                SourceTime::Milliseconds(100),
                Some(SourceTime::Milliseconds(100)),
            )
            .unwrap();
        let fired = table.advance(&registry, SourceTime::Milliseconds(100));
        assert_eq!(fired.len(), 1);
        assert_eq!(table.len(), 1);
        let fired = table.advance(&registry, SourceTime::Milliseconds(150));
        assert!(fired.is_empty());
        let fired = table.advance(&registry, SourceTime::Milliseconds(200));
        assert_eq!(fired.len(), 1);
    }

    #[test]
    fn mixed_units_and_stale_actors_rejected() {
        let (registry, _) = registry();
        let mut table = TimerTable::new();
        let first = registry.observations()[0].id.clone();
        table
            .schedule(&registry, &first, "a", SourceTime::Milliseconds(100), None)
            .unwrap();
        assert_eq!(
            table.schedule(&registry, &first, "b", SourceTime::Seconds(1.0), None),
            Err(WorldError::TimerUnit)
        );
        let owner = IdentityOwner::create("other").unwrap();
        let stale = owner.actor(9, 1);
        assert_eq!(
            table.schedule(&registry, &stale, "c", SourceTime::Milliseconds(100), None),
            Err(WorldError::StaleActor)
        );
    }

    #[test]
    fn timer_checkpoint_round_trip() {
        let (registry, _) = registry();
        let mut table = TimerTable::new();
        let first = registry.observations()[0].id.clone();
        table
            .schedule(&registry, &first, "think", SourceTime::Seconds(1.5), None)
            .unwrap();
        let saved = table.checkpoint();
        let mut restored = TimerTable::new();
        restored.restore(&registry, &saved).unwrap();
        let fired = restored.advance(&registry, SourceTime::Seconds(1.5));
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].name, "think");
    }

    #[test]
    fn powerup_timer_expires() {
        let mut timer = PowerupTimer {
            item: "q3:quad".to_string(),
            label: "Quad".to_string(),
            remaining_seconds: 30.0,
        };
        assert!(timer.tick(29.0));
        assert!(!timer.tick(1.0));
    }
}
