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

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use qa_core::identity::{ActorId, SavedActorId};
use qa_core::math::Vec3;

use super::mod_presentation_checkpoint::{read_saved_actor_id, SourcePlayerState};
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
    let actors = RefCell::new(HashSet::new());
    let order_set = RefCell::new(HashSet::new());
    let read_order = |value: &ProfileReader<'_>| -> Result<u64, GuestError> {
        let result = value.integer(0)? as u64;
        if result >= next_order || !order_set.borrow_mut().insert(result) {
            return value.fail("invalid player event publication order");
        }
        Ok(result)
    };
    let clients = reader.field("clients")?.list(|value| {
        let actor = read_saved_actor_id(&value.field("actor")?)?;
        if !actors.borrow_mut().insert((actor.slot, actor.generation)) {
            return value.fail("duplicate player event cursor");
        }
        let sequence = read_int32(&value.field("sequence")?)?;
        let observed = read_int32(&value.field("observedSequence")?)?;
        let predictable = value.field("predictable")?.list(|entry| {
            Ok((
                read_int32(&entry.field("sequence")?)?,
                read_order(&entry.field("order")?)?,
            ))
        })?;
        let sequences: HashSet<i32> = predictable.iter().map(|(sequence, _)| *sequence).collect();
        if predictable.len() > 2
            || sequences.len() != predictable.len()
            || predictable.iter().any(|(sequence, _)| {
                let delta = observed.wrapping_sub(*sequence);
                !(1..=2).contains(&delta)
            })
        {
            return value.fail("invalid predictable player event cursor");
        }
        Ok(SavedCursor {
            actor,
            external: read_int32(&value.field("external")?)?,
            external_time: read_int32(&value.field("externalTime")?)?,
            sequence,
            observed_sequence: observed,
            external_order: value.field("externalOrder")?.nullable(|order| read_order(order))?,
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
        Self {
            memory,
            ops,
            module,
            abi,
            entries: HashMap::new(),
            next_order: 0,
        }
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
        let order = self
            .next_order
            .checked_add(1)
            .ok_or_else(|| GuestError::invalid("Player event publication sequence exhausted"))?;
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
            return Err(GuestError::invalid(
                "Player event cursor differs from restored source sequence",
            ));
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
        let Some(entry) = self.entries.get(actor) else {
            return Ok(());
        };
        let address = entry.address;
        if offsets.contains(&108) {
            let next = self.memory.read_i32(address + 108)?;
            let cursor = &mut self.entries.get_mut(actor).expect("checked entry").cursor;
            let delta = next.wrapping_sub(cursor.observed_sequence);
            if delta < 0 {
                cursor.sequence = next;
                cursor.predictable.clear();
            } else if delta > 0 {
                cursor
                    .predictable
                    .retain(|sequence, _| next.wrapping_sub(*sequence) <= 2);
                for step in (1..=delta.min(2)).rev() {
                    let order = self.ordinal()?;
                    self.entries
                        .get_mut(actor)
                        .expect("checked entry")
                        .cursor
                        .predictable
                        .insert(next.wrapping_sub(step), order);
                }
            }
            self.entries
                .get_mut(actor)
                .expect("checked entry")
                .cursor
                .observed_sequence = next;
        }
        if offsets.contains(&128) {
            let order = self.ordinal()?;
            self.entries
                .get_mut(actor)
                .expect("checked entry")
                .cursor
                .external_order = Some(order);
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
            let external = ps.external_event() != 0
                && (ps.external_event() != cursor.external || ps.external_event_time() != cursor.external_time);
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
                    pending.push((
                        order,
                        SourcePlayerEvent {
                            actor: actor.clone(),
                            module: self.module.clone(),
                            abi: self.abi,
                            player_state: ps.clone(),
                            origin,
                            time,
                            event: ps.external_event(),
                            parameter: ps.external_event_param(),
                            sequence: PlayerEventSequence::External {
                                time: ps.external_event_time(),
                            },
                        },
                    ));
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
                        pending.push((
                            order,
                            SourcePlayerEvent {
                                actor: actor.clone(),
                                module: self.module.clone(),
                                abi: self.abi,
                                player_state: ps.clone(),
                                origin,
                                time,
                                event,
                                parameter: ps.event_parameters()[slot],
                                sequence: PlayerEventSequence::Predictable { sequence },
                            },
                        ));
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
                    predictable: entry
                        .cursor
                        .predictable
                        .iter()
                        .map(|(sequence, order)| (*sequence, *order))
                        .collect(),
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
            let cursor = saved.and_then(|saved| {
                saved
                    .clients
                    .iter()
                    .find(|entry| reference_saved(entry.actor).as_ref() == Some(actor))
            });
            if saved.is_some() && cursor.is_none() {
                return Err(GuestError::invalid("Missing restored player event cursor"));
            }
            self.track(actor.clone(), *address, cursor)?;
        }
        Ok(())
    }

    /// Stop tracking one actor.
    pub fn release(&mut self, actor: &ActorId) {
        self.entries.remove(actor);
    }

    /// Stop tracking all actors.
    pub fn close(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::IdentityOwner;

    use super::super::mod_presentation_checkpoint::capture_saved_actor_id;
    use super::super::mod_provider::{ProfileValue, QvmAbi};
    use super::*;

    struct FixtureMemory {
        states: HashMap<usize, SourcePlayerState>,
    }

    impl FixtureMemory {
        fn with_state(address: usize, state: SourcePlayerState) -> Self {
            let mut states = HashMap::new();
            states.insert(address, state);
            Self { states }
        }
    }

    impl PlayerEventMemory for FixtureMemory {
        fn read_player_state(&self, address: usize) -> Result<SourcePlayerState, GuestError> {
            self.states
                .get(&address)
                .cloned()
                .ok_or_else(|| GuestError::invalid("missing player record"))
        }

        fn read_i32(&self, address: usize) -> Result<i32, GuestError> {
            for (base, state) in &self.states {
                if address >= *base && address + 4 <= *base + state.bytes().len() {
                    let offset = address - base;
                    let word = &state.bytes()[offset..offset + 4];
                    return Ok(i32::from_le_bytes([word[0], word[1], word[2], word[3]]));
                }
            }
            Err(GuestError::invalid("missing source word"))
        }
    }

    struct FixtureOps {
        live: HashSet<ActorId>,
        emitted: Vec<SourcePlayerEvent>,
    }

    impl FixtureOps {
        fn with_live(actor: &ActorId) -> Self {
            let mut live = HashSet::new();
            live.insert(actor.clone());
            Self {
                live,
                emitted: Vec::new(),
            }
        }
    }

    impl PlayerEventOps for FixtureOps {
        fn live(&self, actor: &ActorId) -> bool {
            self.live.contains(actor)
        }

        fn origin(&self, _actor: &ActorId) -> Result<Vec3, GuestError> {
            Ok(Vec3::default())
        }

        fn time_ms(&self) -> i32 {
            500
        }

        fn emit(&mut self, event: SourcePlayerEvent) {
            self.emitted.push(event);
        }
    }

    fn module() -> ModuleId {
        ModuleId {
            id: "test:game".to_string(),
            artifact_path: "vm/qagame.qvm".to_string(),
            digest: "sha256:game".to_string(),
            revision: "1".to_string(),
        }
    }

    fn write_word(state: &mut SourcePlayerState, offset: usize, value: i32) {
        state.bytes_mut()[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn player_state(sequence: i32, external: i32, external_time: i32) -> SourcePlayerState {
        let mut state = SourcePlayerState::zeroed(QvmAbi::Modern);
        write_word(&mut state, 108, sequence);
        write_word(&mut state, 128, external);
        write_word(&mut state, 136, external_time);
        state
    }

    fn tracker(
        actor: &ActorId,
        address: usize,
        state: SourcePlayerState,
    ) -> QvmPlayerEvents<FixtureMemory, FixtureOps> {
        let mut tracker = QvmPlayerEvents::new(
            FixtureMemory::with_state(address, state),
            FixtureOps::with_live(actor),
            module(),
            QvmAbi::Modern,
        );
        tracker.track(actor.clone(), address, None).expect("track");
        tracker
    }

    #[test]
    fn predictable_event_publishes_observed_order() {
        let owner = IdentityOwner::create("test").expect("owner");
        let actor = owner.actor(3, 0);
        let mut tracker = tracker(&actor, 0x1000, player_state(10, 0, 0));
        let mut advanced = player_state(11, 0, 0);
        write_word(&mut advanced, 112, 7);
        write_word(&mut advanced, 120, 9);
        tracker.memory_mut().states.insert(0x1000, advanced);
        tracker.notify_write(&actor, &[108]).expect("notify");
        tracker.publish().expect("publish");
        let emitted = &tracker.ops().emitted;
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].event, 7);
        assert_eq!(emitted[0].parameter, 9);
        assert_eq!(emitted[0].sequence, PlayerEventSequence::Predictable { sequence: 10 });
        tracker.publish().expect("republish");
        assert_eq!(tracker.ops().emitted.len(), 1);
    }

    #[test]
    fn external_event_publishes_time_identity() {
        let owner = IdentityOwner::create("test").expect("owner");
        let actor = owner.actor(3, 0);
        let mut tracker = tracker(&actor, 0x1000, player_state(10, 0, 0));
        tracker.memory_mut().states.insert(0x1000, player_state(10, 5, 100));
        tracker.notify_write(&actor, &[128]).expect("notify");
        tracker.publish().expect("publish");
        let emitted = &tracker.ops().emitted;
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].event, 5);
        assert_eq!(emitted[0].sequence, PlayerEventSequence::External { time: 100 });
    }

    #[test]
    fn discard_syncs_without_publishing() {
        let owner = IdentityOwner::create("test").expect("owner");
        let actor = owner.actor(3, 0);
        let mut tracker = tracker(&actor, 0x1000, player_state(10, 0, 0));
        tracker.memory_mut().states.insert(0x1000, player_state(12, 5, 100));
        tracker.notify_write(&actor, &[108, 128]).expect("notify");
        tracker.discard().expect("discard");
        tracker.publish().expect("publish");
        assert!(tracker.ops().emitted.is_empty());
    }

    #[test]
    fn checkpoint_restore_round_trip_stays_quiet() {
        let owner = IdentityOwner::create("test").expect("owner");
        let actor = owner.actor(3, 0);
        let mut tracker = tracker(&actor, 0x1000, player_state(10, 0, 0));
        let mut advanced = player_state(11, 0, 0);
        write_word(&mut advanced, 112, 7);
        tracker.memory_mut().states.insert(0x1000, advanced.clone());
        tracker.notify_write(&actor, &[108]).expect("notify");
        tracker.publish().expect("publish");
        assert_eq!(tracker.ops().emitted.len(), 1);
        let saved = tracker.checkpoint();
        assert_eq!(saved.next_order, 1);
        assert_eq!(saved.clients.len(), 1);
        let mut restored = QvmPlayerEvents::new(
            FixtureMemory::with_state(0x1000, advanced),
            FixtureOps::with_live(&actor),
            module(),
            QvmAbi::Modern,
        );
        let reference = |saved: SavedActorId| {
            if saved.slot == actor.slot() && saved.generation == actor.generation() {
                Some(actor.clone())
            } else {
                None
            }
        };
        restored
            .restore(Some(&saved), &[(actor.clone(), 0x1000)], &reference)
            .expect("restore");
        restored.publish().expect("publish");
        assert!(restored.ops().emitted.is_empty());
        assert_eq!(restored.checkpoint(), saved);
    }

    #[test]
    fn restore_rejects_players_without_saved_cursors() {
        let owner = IdentityOwner::create("test").expect("owner");
        let actor = owner.actor(3, 0);
        let other = owner.actor(4, 0);
        let tracker = tracker(&actor, 0x1000, player_state(10, 0, 0));
        let saved = tracker.checkpoint();
        let mut restored = QvmPlayerEvents::new(
            FixtureMemory::with_state(0x2000, player_state(10, 0, 0)),
            FixtureOps::with_live(&other),
            module(),
            QvmAbi::Modern,
        );
        let reference = |saved: SavedActorId| {
            if saved.slot == other.slot() && saved.generation == other.generation() {
                Some(other.clone())
            } else {
                None
            }
        };
        assert!(restored
            .restore(Some(&saved), &[(other.clone(), 0x2000)], &reference)
            .is_err());
    }

    #[test]
    fn release_stops_tracking() {
        let owner = IdentityOwner::create("test").expect("owner");
        let actor = owner.actor(3, 0);
        let other = owner.actor(4, 0);
        let mut tracker = tracker(&actor, 0x1000, player_state(10, 0, 0));
        tracker.memory_mut().states.insert(0x2000, player_state(10, 0, 0));
        tracker.ops.live.insert(other.clone());
        tracker.track(other.clone(), 0x2000, None).expect("track other");
        tracker.release(&actor);
        let mut advanced = player_state(11, 0, 0);
        write_word(&mut advanced, 112, 7);
        tracker.memory_mut().states.insert(0x2000, advanced);
        tracker.notify_write(&other, &[108]).expect("notify");
        tracker.publish().expect("publish");
        let emitted = &tracker.ops().emitted;
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].actor, other);
    }

    fn checkpoint_value(
        next_order: i64,
        actor: &ActorId,
        sequence: i64,
        observed: i64,
        external_order: ProfileValue,
        predictable: ProfileValue,
    ) -> ProfileValue {
        ProfileValue::record(vec![
            ("nextOrder", ProfileValue::Int(next_order)),
            (
                "clients",
                ProfileValue::Array(vec![ProfileValue::record(vec![
                    ("actor", capture_saved_actor_id(actor)),
                    ("external", ProfileValue::Int(0)),
                    ("externalTime", ProfileValue::Int(0)),
                    ("sequence", ProfileValue::Int(sequence)),
                    ("observedSequence", ProfileValue::Int(observed)),
                    ("externalOrder", external_order),
                    ("predictable", predictable),
                ])]),
            ),
        ])
    }

    #[test]
    fn read_checkpoint_validates_publication_orders() {
        let owner = IdentityOwner::create("test").expect("owner");
        let actor = owner.actor(3, 0);
        let entry = ProfileValue::record(vec![
            ("sequence", ProfileValue::Int(11)),
            ("order", ProfileValue::Int(0)),
        ]);
        let value = checkpoint_value(
            2,
            &actor,
            10,
            12,
            ProfileValue::Int(1),
            ProfileValue::Array(vec![entry]),
        );
        let read = read_qvm_player_events(&ProfileReader::new(&value))
            .expect("read")
            .expect("present");
        assert_eq!(read.next_order, 2);
        assert_eq!(read.clients.len(), 1);
        assert_eq!(read.clients[0].external_order, Some(1));
        let duplicate = ProfileValue::Array(vec![
            ProfileValue::record(vec![
                ("sequence", ProfileValue::Int(11)),
                ("order", ProfileValue::Int(0)),
            ]),
            ProfileValue::record(vec![
                ("sequence", ProfileValue::Int(10)),
                ("order", ProfileValue::Int(0)),
            ]),
        ]);
        let bad = checkpoint_value(2, &actor, 10, 12, ProfileValue::Null, duplicate);
        assert!(read_qvm_player_events(&ProfileReader::new(&bad)).is_err());
        assert!(read_qvm_player_events(&ProfileReader::new(&ProfileValue::Undefined))
            .expect("absent")
            .is_none());
    }
}
