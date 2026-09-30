//! QVM player-event cursors: ordered publication of source player events.
//!
//! Provenance: `src/compat/qvm/mod-player-events.ts`.
//!
//! Local mirrors: [`SourcePlayerEvent`] (mirror of `Q3SourcePlayerEvent`),
//! [`PlayerEventMemory`] (the `QvmMemory` slice this tracker needs: player
//! records plus word reads; write observation arrives through
//! [`QvmPlayerEvents::notify_write`], mirroring the donor's committed-memory
//! listener). Player records reuse
//! [`super::mod_presentation_checkpoint::SourcePlayerState`]; [`ModuleId`] and
//! [`QvmAbi`] reuse [`super::mod_provider`].

use std::collections::{HashMap, HashSet};

use qa_core::identity::{ActorId, SavedActorId};
use qa_core::math::Vec3;

use super::mod_presentation_checkpoint::{SourcePlayerState, read_saved_actor_id};
use super::mod_provider::{ModuleId, ProfileReader, QvmAbi};
use crate::error::GuestError;

/// Player-state words this tracker reads.
pub trait PlayerEventMemory {
    /// Read a full player record.
    fn read_player_state(&self, address: usize) -> Result<SourcePlayerState, GuestError>;
    /// Read one source word.
    fn read_i32(&self, address: usize) -> Result<i32, GuestError>;
}

/// Host operations of the tracker.
pub trait PlayerEventOps {
    /// Whether an actor is live.
    fn live(&self, actor: &ActorId) -> bool;
    /// Actor origin.
    fn origin(&self, actor: &ActorId) -> Result<Vec3, GuestError>;
    /// Caller time in milliseconds.
    fn time_ms(&self) -> i32;
    /// Emit a published event.
    fn emit(&mut self, event: SourcePlayerEvent);
}

/// Event sequence identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerEventSequence {
    /// External (non-predicted) event at a time.
    External {
        /// Event time in milliseconds.
        time: i32,
    },
    /// Predictable event at a sequence.
    Predictable {
        /// Sequence number.
        sequence: i32,
    },
}

/// Published source player event (mirror of `Q3SourcePlayerEvent`).
#[derive(Debug, Clone)]
pub struct SourcePlayerEvent {
    /// Owning actor.
    pub actor: ActorId,
    /// Gameplay module.
    pub module: ModuleId,
    /// ABI profile.
    pub abi: QvmAbi,
    /// Player state at publication.
    pub player_state: SourcePlayerState,
    /// Event origin.
    pub origin: Vec3,
    /// Publication time in milliseconds.
    pub time: i32,
    /// Event id.
    pub event: i32,
    /// Event parameter.
    pub parameter: i32,
    /// Sequence identity.
    pub sequence: PlayerEventSequence,
}

#[derive(Debug, Clone)]
struct Cursor {
    external: i32,
    external_time: i32,
    sequence: i32,
    observed_sequence: i32,
    external_order: Option<u64>,
    predictable: HashMap<i32, u64>,
}

#[derive(Debug, Clone)]
struct Entry {
    actor: ActorId,
    address: usize,
    cursor: Cursor,
}

/// Saved cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedCursor {
    /// Actor.
    pub actor: SavedActorId,
    /// Last external event.
    pub external: i32,
    /// Last external event time.
    pub external_time: i32,
    /// Published sequence.
    pub sequence: i32,
    /// Observed sequence.
    pub observed_sequence: i32,
    /// External publication order, if any.
    pub external_order: Option<u64>,
    /// Predictable orders.
    pub predictable: Vec<(i32, u64)>,
}

/// Player-events checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmPlayerEventsCheckpoint {
    /// Next publication order.
    pub next_order: u64,
    /// Saved cursors.
    pub clients: Vec<SavedCursor>,
}

fn read_int32(reader: &ProfileReader<'_>) -> Result<i32, GuestError> {
    let value = reader.integer(i64::from(i32::MIN))?;
    if value > i64::from(i32::MAX) {
        return reader.fail("expected a source int32");
    }
    Ok(value as i32)
}

/// Read a player-events checkpoint (absent reads as `None`).
pub fn read_qvm_player_events(reader: &ProfileReader<'_>) -> Result<Option<QvmPlayerEventsCheckpoint>, GuestError> {
    if reader.is_undefined() {
        return Ok(None);
    }
    let next_order = reader.field("nextOrder")?.integer(0)? as u64;
    let mut actors = HashSet::new();
    let mut order_set = HashSet::new();
    let read_order = |value: &ProfileReader<'_>, orders: &mut HashSet<u64>| -> Result<u64, GuestError> {
        let result = value.integer(0)? as u64;
        if result >= next_order || !orders.insert(result) {
            return value.fail("invalid player event publication order");
        }
        Ok(result)
    };
    let clients = reader.field("clients")?.list(|value| {
        let actor = read_saved_actor_id(&value.field("actor")?)?;
        if !actors.insert((actor.slot, actor.generation)) {
            return value.fail("duplicate player event cursor");
        }
        let sequence = read_int32(&value.field("sequence")?)?;
        let observed = read_int32(&value.field("observedSequence")?)?;
        let predictable = value.field("predictable")?.list(|entry| {
            Ok((read_int32(&entry.field("sequence")?)?, read_order(&entry.field("order")?, &mut order_set)?))
        })?;
        let sequences: HashSet<i32> = predictable.iter().map(|(sequence, _)| *sequence).collect();
        if predictable.len() > 2
            || sequences.len() != predictable.len()
            || predictable.iter().any(|(sequence, _)| {
                let delta = observed.wrapping_sub(*sequence);
                !(1..=2).contains(&delta)
            }) {
            return value.fail("invalid predictable player event cursor");
        }
        Ok(SavedCursor {
            actor,
            external: read_int32(&value.field("external")?)?,
            external_time: read_int32(&value.field("externalTime")?)?,
            sequence,
            observed_sequence: observed,
            external_order: value.field("externalOrder")?.nullable(|order| read_order(order, &mut order_set))?,
            predictable,
        })
    })?;
    Ok(Some(QvmPlayerEventsCheckpoint { next_order, clients }))
}

/// Observes source publication order without running host operations.
pub struct QvmPlayerEvents<M: PlayerEventMemory, O: PlayerEventOps> {
    memory: M,
    ops: O,
    module: ModuleId,
    abi: QvmAbi,
    entries: HashMap<ActorId, Entry>,
    next_order: u64,
}

impl<M: PlayerEventMemory, O: PlayerEventOps> QvmPlayerEvents<M, O> {
    /// Create a tracker.
    pub fn new(memory: M, ops: O, module: ModuleId, abi: QvmAbi) -> Self {
        Self { memory, ops, module, abi, entries: HashMap::new(), next_order: 0 }
    }

    /// Borrow the memory.
    pub fn memory(&self) -> &M {
        &self.memory
    }

    /// Mutably borrow the memory.
    pub fn memory_mut(&mut self) -> &mut M {
        &mut self.memory
    }

    /// Borrow the operations.
    pub fn ops(&self) -> &O {
        &self.ops
    }

    fn ordinal(&mut self) -> Result<u64, GuestError> {
        let order = self.next_order.checked_add(1).ok_or_else(|| GuestError::invalid("Player event publication sequence exhausted"))?;
        self.next_order = order;
        Ok(order - 1)
    }

    /// Track a player record, optionally from a saved cursor.
    pub fn track(&mut self, actor: ActorId, address: usize, saved: Option<&SavedCursor>) -> Result<(), GuestError> {
        if self.entries.contains_key(&actor) {
            return Ok(());
        }
        let sequence = self.memory.read_i32(address + 108)?;
        if saved.is_some_and(|saved| saved.observed_sequence != sequence) {
            return Err(GuestError::invalid("Player event cursor differs from restored source sequence"));
        }
        let cursor = match saved {
            None => Cursor {
                external: self.memory.read_i32(address + 128)?,
                external_time: self.memory.read_i32(address + 136)?,
                sequence,
                observed_sequence: sequence,
                external_order: None,
                predictable: HashMap::new(),
            },
            Some(saved) => Cursor {
                external: saved.external,
                external_time: saved.external_time,
                sequence: saved.sequence,
                observed_sequence: saved.observed_sequence,
                external_order: saved.external_order,
                predictable: saved.predictable.iter().copied().collect(),
            },
        };
        self.entries.insert(actor.clone(), Entry { actor, address, cursor });
        Ok(())
    }

    /// Observe source writes touching `offsets` (player-record relative).
    pub fn notify_write(&mut self, actor: &ActorId, offsets: &[usize]) -> Result<(), GuestError> {
        let Some(entry) = self.entries.get(actor) else { return Ok(()) };
        let address = entry.address;
        if offsets.contains(&108) {
            let next = self.memory.read_i32(address + 108)?;
            let cursor = &mut self.entries.get_mut(actor).expect("checked entry").cursor;
            let delta = next.wrapping_sub(cursor.observed_sequence);
            if delta < 0 {
                cursor.sequence = next;
                cursor.predictable.clear();
            } else if delta > 0 {
                cursor.predictable.retain(|sequence, _| next.wrapping_sub(*sequence) <= 2);
                for step in (1..=delta.min(2)).rev() {
                    let order = self.ordinal()?;
                    self.entries.get_mut(actor).expect("checked entry").cursor.predictable.insert(next.wrapping_sub(step), order);
                }
            }
            self.entries.get_mut(actor).expect("checked entry").cursor.observed_sequence = next;
        }
        if offsets.contains(&128) {
            let order = self.ordinal()?;
            self.entries.get_mut(actor).expect("checked entry").cursor.external_order = Some(order);
        }
        Ok(())
    }

    /// Publish pending events in source order.
    pub fn publish(&mut self) -> Result<(), GuestError> {
        let actors: Vec<ActorId> = self.entries.keys().cloned().collect();
        let mut pending: Vec<(u64, SourcePlayerEvent)> = Vec::new();
        for actor in &actors {
            let entry = self.entries.get(actor).expect("tracked actor");
            if !self.ops.live(actor) {
                continue;
            }
            let address = entry.address;
            let ps = self.memory.read_player_state(address)?;
            let cursor = &self.entries.get(actor).expect("tracked actor").cursor;
            let external = ps.external_event() != 0 && (ps.external_event() != cursor.external || ps.external_event_time() != cursor.external_time);
            let count = (ps.event_sequence().wrapping_sub(cursor.sequence)).clamp(0, 2);
            if external || count != 0 {
                let origin = self.ops.origin(actor)?;
                let time = self.ops.time_ms();
                if external {
                    let order = cursor.external_order;
                    let order = match order {
                        Some(order) => order,
                        None => self.ordinal()?,
                    };
                    pending.push((order, SourcePlayerEvent {
                        actor: actor.clone(),
                        module: self.module.clone(),
                        abi: self.abi,
                        player_state: ps.clone(),
                        origin,
                        time,
                        event: ps.external_event(),
                        parameter: ps.external_event_param(),
                        sequence: PlayerEventSequence::External { time: ps.external_event_time() },
                    }));
                }
                for step in (1..=count).rev() {
                    let sequence = ps.event_sequence().wrapping_sub(step);
                    let slot = (sequence & 1) as usize;
                    let event = ps.events()[slot];
                    if event != 0 {
                        let cursor = &self.entries.get(actor).expect("tracked actor").cursor;
                        let order = cursor.predictable.get(&sequence).copied();
                        let order = match order {
                            Some(order) => order,
                            None => self.ordinal()?,
                        };
                        pending.push((order, SourcePlayerEvent {
                            actor: actor.clone(),
                            module: self.module.clone(),
                            abi: self.abi,
                            player_state: ps.clone(),
                            origin,
                            time,
                            event,
                            parameter: ps.event_parameters()[slot],
                            sequence: PlayerEventSequence::Predictable { sequence },
                        }));
                    }
                }
            }
            let cursor = &mut self.entries.get_mut(actor).expect("tracked actor").cursor;
            cursor.external = ps.external_event();
            cursor.external_time = ps.external_event_time();
            cursor.sequence = ps.event_sequence();
            cursor.observed_sequence = ps.event_sequence();
            cursor.external_order = None;
            cursor.predictable.clear();
        }
        pending.sort_by_key(|(order, _)| *order);
        for (_, event) in pending {
            if self.entries.contains_key(&event.actor) && self.ops.live(&event.actor) {
                self.ops.emit(event);
            }
        }
        Ok(())
    }

    /// Discard pending observations without publishing.
    pub fn discard(&mut self) -> Result<(), GuestError> {
        let actors: Vec<ActorId> = self.entries.keys().cloned().collect();
        for actor in &actors {
            let address = self.entries.get(actor).expect("tracked actor").address;
            let external = self.memory.read_i32(address + 128)?;
            let external_time = self.memory.read_i32(address + 136)?;
            let sequence = self.memory.read_i32(address + 108)?;
            let cursor = &mut self.entries.get_mut(actor).expect("tracked actor").cursor;
            cursor.external = external;
            cursor.external_time = external_time;
            cursor.sequence = sequence;
            cursor.observed_sequence = sequence;
            cursor.external_order = None;
            cursor.predictable.clear();
        }
        Ok(())
    }

    /// Capture a checkpoint.
    #[must_use]
    pub fn checkpoint(&self) -> QvmPlayerEventsCheckpoint {
        QvmPlayerEventsCheckpoint {
            next_order: self.next_order,
            clients: self
                .entries
                .values()
                .map(|entry| SavedCursor {
                    actor: SavedActorId::from(&entry.actor),
                    external: entry.cursor.external,
                    external_time: entry.cursor.external_time,
                    sequence: entry.cursor.sequence,
                    observed_sequence: entry.cursor.observed_sequence,
                    external_order: entry.cursor.external_order,
                    predictable: entry.cursor.predictable.iter().map(|(sequence, order)| (*sequence, *order)).collect(),
                })
                .collect(),
        }
    }

    /// Restore tracking from a checkpoint.
    pub fn restore(
        &mut self,
        saved: Option<&QvmPlayerEventsCheckpoint>,
        players: &[(ActorId, usize)],
        reference_saved: &dyn Fn(SavedActorId) -> Option<ActorId>,
    ) -> Result<(), GuestError> {
        self.close();
        self.next_order = saved.map_or(0, |saved| saved.next_order);
        for (actor, address) in players {
            let cursor = saved.and_then(|saved| saved.clients.iter().find(|entry| reference_saved(entry.actor).as_ref() == Some(actor)));
            if saved.is_some() && cursor.is_none() {
                return Err(GuestError::invalid("Missing restored player event cursor"));
            }
            self.track(actor.clone(), *address, cursor)?;
        }
        Ok(())
   
...[truncated 6964 chars]