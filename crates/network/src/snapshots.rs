//! Connection-owned native projections. Storage is reserved at connect;
//! snapshot retention never owns another engine player/entity implementation.
use crate::{
    commands::packet,
    message::{Reader, Writer},
    states,
};
mod q2;
pub use q2::{
    Q2EntityPolicy, Q2Header, Q2KexContext, Q2KexFrame, Q2KexRing, Q2ReproFrame, Q2ReproRing,
    read_q2_kex, read_q2_repro, write_q2_kex, write_q2_repro,
};

pub const SLOTS: usize = 32;

/// Convert one native area byte to or from common 1=hidden polarity.
/// The native protocol boundary supplies whether its bits mean visible.
pub const fn native_area_byte(byte: u8, visible: bool) -> u8 {
    byte ^ if visible { u8::MAX } else { 0 }
}

/// Native snapshot storage and usable entity-number boundaries selected once
/// by the connection protocol, never inferred from common reservations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotLimits {
    pub native_numbers: usize,
    pub entity_capacity: usize,
    pub area_capacity: usize,
    pub first_entity: u32,
    pub end_entity: u32,
    retained_rows: Option<u64>,
}
pub const fn snapshot_limits(protocol: packet::Protocol) -> SnapshotLimits {
    let (native_numbers, entity_capacity, area_capacity, first_entity, end_entity, retained_rows) =
        match protocol {
            packet::Protocol::NetQuake15 => (600, 600, 0, 1, 600, None),
            packet::Protocol::QuakeWorld28 => (512, 64, 0, 1, 512, None),
            packet::Protocol::Quake2_34 => (1024, 1023, 32, 1, 1024, Some(896)),
            packet::Protocol::Quake2Repro1038 => (8192, 8191, 255, 1, 8192, Some(8192)),
            packet::Protocol::Quake3_68 => (1024, 1023, 32, 0, 1023, Some(1920)),
        };
    SnapshotLimits {
        native_numbers,
        entity_capacity,
        area_capacity,
        first_entity,
        end_entity,
        retained_rows,
    }
}

const fn slot(sequence: u32) -> usize {
    sequence as usize & (SLOTS - 1)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entity<const N: usize> {
    pub number: u32,
    pub words: [u32; N],
}

/// Submission metadata for native ping and outgoing command redundancy.
#[derive(Clone, Copy)]
struct MoveObservation {
    sequence: Option<u32>,
    time: qa_core::sys_events::EventTime,
    command: crate::channel::MoveCommand,
}
impl MoveObservation {
    const ZERO: Self = Self {
        sequence: None,
        time: qa_core::sys_events::EventTime(0),
        command: crate::channel::MoveCommand {
            number: 0,
            server_time_ms: 0,
        },
    };
}

#[derive(Clone, Copy)]
struct Slot<const P: usize> {
    sequence: Option<u32>,
    observation: Option<qa_core::primitives::SnapshotObservation>,
    request_sequence: Option<u32>,
    requested_base: Option<u32>,
    sent_move: MoveObservation,
    ping: i32,
    valid: bool,
    time: qa_core::primitives::ThinkTime,
    command: u32,
    flags: u8,
    area_bytes: usize,
    count: usize,
    first_entity: u64,
    qw_peers: u32,
    player: [u32; P],
}
impl<const P: usize> Slot<P> {
    const ZERO: Self = Self {
        sequence: None,
        observation: None,
        request_sequence: None,
        requested_base: None,
        sent_move: MoveObservation::ZERO,
        ping: 0,
        valid: false,
        time: qa_core::primitives::ThinkTime::Milliseconds(0),
        command: 0,
        flags: 0,
        area_bytes: 0,
        count: 0,
        first_entity: 0,
        qw_peers: 0,
        player: [0; P],
    };
}

#[derive(Clone, Copy, Debug)]
pub struct Frame<'a, const P: usize, const E: usize> {
    pub sequence: u32,
    pub time: qa_core::primitives::ThinkTime,
    pub command: u32,
    pub flags: u8,
    /// Accepted QW playerinfo rows, carried by the existing native ring slot.
    pub qw_peers: u32,
    /// Common polarity: a set bit hides the area. Native codecs convert at admission/submission.
    pub areas: &'a [u8],
    pub player: &'a [u32; P],
    pub entities: &'a [Entity<E>],
}
impl<const P: usize, const E: usize> Frame<'_, P, E> {
    pub fn qw_player(&self, number: u32) -> bool {
        number
            .checked_sub(1)
            .filter(|&peer| peer < states::QW_PLAYER_SLOTS as u32)
            .is_some_and(|peer| self.qw_peers & (1 << peer) != 0)
    }
}

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Counts {
    pub accepted: u64,
    pub missing_base: u64,
    pub overflow: u64,
}

pub struct Ring<const P: usize, const E: usize> {
    slots: [Slot<P>; SLOTS],
    entities: Box<[Entity<E>]>,
    areas: Box<[u8]>,
    baselines: Box<[[u32; E]]>,
    scratch: Box<[Entity<E>]>,
    scratch_areas: Box<[u8]>,
    capacity: usize,
    area_capacity: usize,
    retained_rows: Option<u64>,
    parsed_rows: u64,
    latest: Option<u32>,
    previous: Option<u32>,
    counts: Counts,
}

impl<const P: usize, const E: usize> Ring<P, E> {
    pub(crate) fn record_move(
        &mut self,
        sequence: u32,
        time: qa_core::sys_events::EventTime,
        command: crate::channel::MoveCommand,
    ) {
        self.slots[slot(sequence)].sent_move = MoveObservation {
            sequence: Some(sequence),
            time,
            command,
        };
    }
    pub(crate) fn submitted_command(&self, sequence: u32) -> Option<crate::channel::MoveCommand> {
        let sent = self.slots[slot(sequence)].sent_move;
        (sent.sequence == Some(sequence)).then_some(sent.command)
    }
    pub fn load(
        capacity: usize,
        native_numbers: usize,
        area_capacity: usize,
        retained_rows: Option<u64>,
    ) -> Result<Self, packet::Error> {
        let rows = capacity.checked_mul(SLOTS).ok_or(packet::Error::Count)?;
        let areas = area_capacity
            .checked_mul(SLOTS)
            .ok_or(packet::Error::Count)?;
        if capacity == 0
            || capacity > native_numbers
            || native_numbers > 65536
            || area_capacity > 255
        {
            return Err(packet::Error::Count);
        }
        let empty = Entity {
            number: 0,
            words: [0; E],
        };
        Ok(Self {
            slots: [Slot::ZERO; SLOTS],
            entities: vec![empty; rows].into_boxed_slice(),
            areas: vec![0; areas].into_boxed_slice(),
            baselines: vec![[0; E]; native_numbers].into_boxed_slice(),
            scratch: vec![empty; capacity].into_boxed_slice(),
            scratch_areas: vec![0; area_capacity].into_boxed_slice(),
            capacity,
            area_capacity,
            retained_rows,
            parsed_rows: 0,
            latest: None,
            previous: None,
            counts: Counts::default(),
        })
    }
    pub fn counts(&self) -> Counts {
        self.counts
    }
    /// Association for the native reply to a submitted client packet. These
    /// scalars are protocol metadata, never retained input or another frame.
    pub(crate) fn record_request(&mut self, sequence: u32, base: Option<u32>) {
        let slot = &mut self.slots[slot(sequence)];
        slot.request_sequence = Some(sequence);
        slot.requested_base = base;
    }
    pub(crate) fn requested_base(&self, sequence: u32) -> Option<Option<u32>> {
        let slot = &self.slots[slot(sequence)];
        (slot.request_sequence == Some(sequence)).then_some(slot.requested_base)
    }
    fn observe_submission(
        &mut self,
        sequence: u32,
        observation: Option<qa_core::primitives::SnapshotObservation>,
    ) {
        let index = slot(sequence);
        let slot = &self.slots[index];
        if slot.valid && slot.sequence == Some(sequence) && !self.expired(slot) {
            self.slots[index].observation = observation;
        }
    }
    fn observation(
        &self,
        sequence: u32,
        mask: u32,
    ) -> Option<qa_core::primitives::SnapshotObservation> {
        let slot = &self.slots[slot(sequence)];
        if !slot.valid || (slot.sequence? & mask) != sequence || self.expired(slot) {
            return None;
        }
        slot.observation
    }
    pub fn allocated_bytes(&self) -> usize {
        std::mem::size_of_val(&self.slots)
            + std::mem::size_of_val(&*self.entities)
            + std::mem::size_of_val(&*self.areas)
            + std::mem::size_of_val(&*self.baselines)
            + std::mem::size_of_val(&*self.scratch)
            + self.scratch_areas.len()
    }
    pub fn set_baseline(&mut self, number: u32, words: &[u32; E]) -> bool {
        // A gamestate establishes these before any frame is decoded/published.
        if self.latest.is_some() || self.parsed_rows != 0 {
            return false;
        }
        let Some(to) = self.baselines.get_mut(number as usize) else {
            return false;
        };
        *to = *words;
        true
    }
    fn expired(&self, slot: &Slot<P>) -> bool {
        self.retained_rows
            .is_some_and(|n| self.parsed_rows.saturating_sub(slot.first_entity) > n)
    }
    pub fn frame(&self, sequence: u32) -> Option<Frame<'_, P, E>> {
        self.retained_frame(sequence, self.expired(&self.slots[self::slot(sequence)]))
    }
    fn retained_frame(&self, sequence: u32, expired: bool) -> Option<Frame<'_, P, E>> {
        let index = self::slot(sequence);
        let slot = &self.slots[index];
        if !slot.valid || slot.sequence != Some(sequence) || expired {
            return None;
        }
        Some(Frame {
            sequence,
            time: slot.time,
            command: slot.command,
            flags: slot.flags,
            qw_peers: slot.qw_peers,
            player: &slot.player,
            areas: rows(&self.areas, self.area_capacity, index, slot.area_bytes),
            entities: rows(&self.entities, self.capacity, index, slot.count),
        })
    }
    /// An invalid Q2 current frame requests a full update, even when an older
    /// accepted frame remains available as a future native delta base.
    pub fn current(&self) -> Option<Frame<'_, P, E>> {
        self.frame(self.latest?)
    }
    /// The prior accepted frame borrows the same bounded ring. An invalid
    /// latest frame never replaces this source, and expired rows return None.
    pub fn previous(&self) -> Option<Frame<'_, P, E>> {
        self.frame(self.previous?)
    }
    /// The provider supplies native numbers and reduced words. Unrepresentable
    /// capabilities are mapped/dropped before this connection boundary.
    pub fn store(&mut self, frame: Frame<'_, P, E>) -> Result<(), packet::Error> {
        if frame.entities.len() > self.capacity
            || frame.areas.len() > self.area_capacity
            || frame
                .entities
                .iter()
                .any(|e| e.number as usize >= self.baselines.len())
            || frame
                .entities
                .windows(2)
                .any(|e| e[0].number >= e[1].number)
        {
            return Err(packet::Error::Count);
        }
        let first_entity = self.parsed_rows;
        self.parsed_rows = self.parsed_rows.saturating_add(frame.entities.len() as u64);
        let slot = Slot {
            sequence: Some(frame.sequence),
            valid: true,
            time: frame.time,
            command: frame.command,
            flags: frame.flags,
            qw_peers: frame.qw_peers,
            area_bytes: frame.areas.len(),
            count: frame.entities.len(),
            first_entity,
            player: *frame.player,
            ..Slot::ZERO
        };
        let index = self::slot(frame.sequence);
        rows_mut(
            &mut self.areas,
            self.area_capacity,
            index,
            frame.areas.len(),
        )
        .copy_from_slice(frame.areas);
        rows_mut(
            &mut self.entities,
            self.capacity,
            index,
            frame.entities.len(),
        )
        .copy_from_slice(frame.entities);
        self.commit(slot);
        Ok(())
    }
    fn commit(&mut self, mut slot: Slot<P>) {
        let Some(sequence) = slot.sequence else {
            return;
        };
        if let Some(latest) = self.latest
            && latest != sequence
            && self.frame(latest).is_some()
        {
            self.previous = Some(latest);
        }
        let start = self
            .latest
            .map_or(1, |n| n.saturating_add(1))
            .max(sequence.saturating_sub((SLOTS - 1) as u32));
        for missing in start..sequence {
            self.slots[self::slot(missing)].sequence = None;
        }
        let index = self::slot(sequence);
        slot.request_sequence = self.slots[index].request_sequence;
        slot.requested_base = self.slots[index].requested_base;
        slot.sent_move = self.slots[index].sent_move;
        self.slots[index] = slot;
        self.latest = Some(sequence);
        self.counts.accepted += u64::from(slot.valid);
    }

    fn publish_received(
        &mut self,
        mut slot: Slot<P>,
        base_valid: bool,
        overflow: bool,
        save_invalid: bool,
    ) -> bool {
        slot.first_entity = self.parsed_rows;
        self.parsed_rows = self.parsed_rows.saturating_add(slot.count as u64);
        if !base_valid {
            self.counts.missing_base += 1;
        }
        if overflow {
            self.counts.overflow += 1;
        }
        if overflow || (!base_valid && !save_invalid) {
            return false;
        }
        let Some(sequence) = slot.sequence else {
            return false;
        };
        let index = self::slot(sequence);
        rows_mut(&mut self.areas, self.area_capacity, index, slot.area_bytes)
            .copy_from_slice(&self.scratch_areas[..slot.area_bytes]);
        rows_mut(&mut self.entities, self.capacity, index, slot.count)
            .copy_from_slice(&self.scratch[..slot.count]);
        slot.valid = base_valid;
        self.commit(slot);
        base_valid
    }
}

pub type Q3Ring = Ring<{ states::PLAYER_WORDS }, { states::ENTITY_WORDS }>;
pub type Q3Frame<'a> = Frame<'a, { states::PLAYER_WORDS }, { states::ENTITY_WORDS }>;

/// Native cgame view borrows the same accepted CLIENT frame and transport metadata.
#[derive(Clone, Copy, Debug)]
pub struct Q3ClientSnapshot<'a> {
    pub frame: Q3Frame<'a>,
    pub ping: i32,
}
impl Q3Ring {
    pub(crate) fn native_numbers(&self) -> usize {
        self.baselines.len()
    }
    /// A validated new gamestate replaces native baselines and accepted frames.
    /// Load-sized storage and cumulative admission counters remain owned here.
    pub(crate) fn reset_gamestate(&mut self) {
        self.slots.fill(Slot::ZERO);
        self.baselines.fill([0; states::ENTITY_WORDS]);
        self.parsed_rows = 0;
        self.latest = None;
        self.previous = None;
    }
    pub(crate) fn client_number(&self) -> (u32, qa_core::primitives::ThinkTime) {
        self.latest.map_or(
            (0, qa_core::primitives::ThinkTime::Milliseconds(0)),
            |sequence| (sequence, self.slots[slot(sequence)].time),
        )
    }
    pub(crate) fn client_snapshot(
        &self,
        sequence: i32,
    ) -> Result<Option<Q3ClientSnapshot<'_>>, packet::Error> {
        let latest = self.latest.unwrap_or(0);
        if i64::from(sequence) > i64::from(latest) {
            return Err(packet::Error::Context);
        }
        let Ok(sequence) = u32::try_from(sequence) else {
            return Ok(None);
        };
        if latest.saturating_sub(sequence) >= SLOTS as u32 {
            return Ok(None);
        }
        let index = slot(sequence);
        let retained = &self.slots[index];
        // CL_GetSnapshot uses MAX_PARSE_ENTITIES, not MSG's delta-base margin.
        let expired = self.parsed_rows.saturating_sub(retained.first_entity) >= 2048;
        Ok(self
            .retained_frame(sequence, expired)
            .map(|frame| Q3ClientSnapshot {
                frame,
                ping: retained.ping,
            }))
    }
    pub(crate) fn observe_ping(
        &mut self,
        sequence: u32,
        queued_at: qa_core::sys_events::EventTime,
        outgoing_sequence: u32,
    ) {
        let index = slot(sequence);
        if !self.slots[index].valid || self.slots[index].sequence != Some(sequence) {
            return;
        }
        let command_time = self.slots[index].player[0] as i32;
        let mut ping = 999;
        for offset in 0..SLOTS as u32 {
            let observation =
                self.slots[slot(outgoing_sequence.wrapping_sub(1 + offset))].sent_move;
            if command_time >= observation.command.server_time_ms {
                // Both operands narrow at the original signed millisecond boundary.
                ping = (queued_at.milliseconds() as i32)
                    .wrapping_sub(observation.time.milliseconds() as i32);
                break;
            }
        }
        self.slots[index].ping = ping;
    }
}

pub type Q2Ring = Ring<{ states::Q2_PLAYER_WORDS }, { states::Q2_ENTITY_WORDS }>;
pub type Q2Frame<'a> = Frame<'a, { states::Q2_PLAYER_WORDS }, { states::Q2_ENTITY_WORDS }>;
/// QW playerinfo is an independent native service, not part of packetentities.
pub type QwRing = Ring<0, { states::QW_ENTITY_WORDS }>;
pub type QwFrame<'a> = Frame<'a, 0, { states::QW_ENTITY_WORDS }>;
pub type NqRing = Ring<{ states::NQ_PLAYER_WORDS }, { states::NQ_ENTITY_WORDS }>;
pub type NqFrame<'a> = Frame<'a, { states::NQ_PLAYER_WORDS }, { states::NQ_ENTITY_WORDS }>;

pub(crate) struct NqStorage {
    pub ring: NqRing,
    pub weapon_is_mask: bool,
    marks: qa_core::stamps::StampSet,
    indices: Box<[usize]>,
    times: Box<[f64]>,
}
pub(crate) struct NqPacket {
    pub time: qa_core::primitives::ThinkTime,
    pub player: [u32; states::NQ_PLAYER_WORDS],
    pub changed: bool,
    count: usize,
    overflow: bool,
}
impl NqStorage {
    fn load(endpoint: qa_core::loopback::Endpoint) -> Result<Self, packet::Error> {
        // Protocol 15 stock CL_EntityNum uses MAX_EDICTS=600. Raised native
        // limits belong to explicit later protocol/capability negotiation.
        let limits = snapshot_limits(packet::Protocol::NetQuake15);
        let scratch = if endpoint == qa_core::loopback::Endpoint::Client {
            limits.native_numbers
        } else {
            0
        };
        Ok(Self {
            ring: NqRing::load(
                limits.entity_capacity,
                limits.native_numbers,
                limits.area_capacity,
                limits.retained_rows,
            )?,
            weapon_is_mask: false,
            marks: qa_core::stamps::StampSet::new(scratch),
            indices: vec![0; scratch].into_boxed_slice(),
            times: vec![0.0; scratch].into_boxed_slice(),
        })
    }
    pub fn begin(&mut self) -> NqPacket {
        self.marks.begin();
        let old = self.ring.latest.map(|n| self.ring.slots[slot(n)]);
        let mut packet = NqPacket {
            time: old.map_or(qa_core::primitives::ThinkTime::Seconds(0.0), |slot| {
                slot.time
            }),
            player: old.map_or([0; states::NQ_PLAYER_WORDS], |slot| slot.player),
            changed: false,
            count: 0,
            overflow: false,
        };
        if let Some(old) = old {
            let index = old.sequence.map_or(0, slot);
            self.ring.scratch[..old.count].copy_from_slice(rows(
                &self.ring.entities,
                self.ring.capacity,
                index,
                old.count,
            ));
            packet.count = old.count;
            let seconds = old.time.seconds();
            for (index, entity) in self.ring.scratch[..old.count].iter().enumerate() {
                self.marks.mark(entity.number as usize);
                self.indices[entity.number as usize] = index;
                self.times[index] = seconds;
            }
        }
        packet
    }
    pub fn entity(
        &mut self,
        reader: &mut Reader<'_>,
        packet: &mut NqPacket,
    ) -> Result<(), packet::Error> {
        let header = states::read_nq_entity_header(reader)?;
        let number = usize::from(header.number);
        let baseline = self
            .ring
            .baselines
            .get(number)
            .ok_or(packet::Error::Count)?;
        let decoded = states::read_nq_entity_body(reader, header, baseline)?;
        let words = decoded.words.ok_or(packet::Error::Context)?;
        let index = if self.marks.test_and_set(number) {
            self.indices[number]
        } else {
            let index = packet.count;
            packet.count += 1;
            self.indices[number] = index;
            index
        };
        if let Some(slot) = self.ring.scratch.get_mut(index) {
            *slot = Entity {
                number: number as u32,
                words,
            };
            self.times[index] = packet.time.seconds();
        } else {
            packet.overflow = true;
        }
        packet.changed = true;
        Ok(())
    }
    pub fn finish(&mut self, sequence: u32, packet: NqPacket) -> Option<u32> {
        if !packet.changed {
            return None;
        }
        let seconds = packet.time.seconds();
        let mut count = 0;
        // CL_RelinkEntities hides records whose msgtime differs from cl.mtime[0].
        // Preserve untouched entities only when the native server time is equal.
        for index in 0..packet.count.min(self.ring.scratch.len()) {
            if self.times[index] == seconds {
                self.ring.scratch[count] = self.ring.scratch[index];
                count += 1;
            }
        }
        self.ring.scratch[..count].sort_unstable_by_key(|entity| entity.number);
        self.ring
            .publish_received(
                Slot {
                    sequence: Some(sequence),
                    time: packet.time,
                    player: packet.player,
                    count,
                    ..Slot::ZERO
                },
                true,
                packet.overflow,
                false,
            )
            .then_some(sequence)
    }
}

/// Native record widths are protocol data; all variants borrow the same Ring
/// implementation and never contain another engine player/entity store.
#[derive(Clone, Copy, Debug)]
pub enum ReceivedFrame<'a> {
    NetQuake(NqFrame<'a>),
    QuakeWorld(QwFrame<'a>),
    Quake2(Q2Frame<'a>),
    Quake2Repro(Q2ReproFrame<'a>),
    Quake3(Q3Frame<'a>),
}
impl<'a> ReceivedFrame<'a> {
    /// QW playerinfo is separate; it does not have a playerstate in this frame.
    pub fn player_words(self) -> Option<&'a [u32]> {
        match self {
            Self::NetQuake(frame) => Some(frame.player),
            Self::QuakeWorld(_) => None,
            Self::Quake2(frame) => Some(frame.player),
            Self::Quake2Repro(frame) => Some(frame.player),
            Self::Quake3(frame) => Some(frame.player),
        }
    }
    pub fn time(self) -> qa_core::primitives::ThinkTime {
        match self {
            Self::NetQuake(frame) => frame.time,
            Self::QuakeWorld(frame) => frame.time,
            Self::Quake2(frame) => frame.time,
            Self::Quake2Repro(frame) => frame.time,
            Self::Quake3(frame) => frame.time,
        }
    }
    pub fn sequence(self) -> u32 {
        match self {
            Self::NetQuake(frame) => frame.sequence,
            Self::QuakeWorld(frame) => frame.sequence,
            Self::Quake2(frame) => frame.sequence,
            Self::Quake2Repro(frame) => frame.sequence,
            Self::Quake3(frame) => frame.sequence,
        }
    }
}

pub(crate) enum Storage {
    NetQuake(Box<NqStorage>),
    QuakeWorld {
        ring: Box<QwRing>,
        // CL_ParsePlayerinfo preserves an omitted PF_COMMAND in its native
        // 64-slot frame. Only that received decoder context is retained here;
        // engine player state and the 32-slot snapshot ring remain independent.
        player_commands: Box<[[crate::commands::QwCmd; states::QW_PLAYER_SLOTS]]>,
        player_model: u32,
    },
    Quake2(Box<Q2Ring>),
    Quake2Repro {
        ring: Box<Q2ReproRing>,
        frame_ms: u16,
        entity_policy: Q2EntityPolicy,
    },
    Quake3(Box<Q3Ring>),
}
impl Storage {
    pub(crate) fn observe_submission(
        &mut self,
        sequence: u32,
        observation: Option<qa_core::primitives::SnapshotObservation>,
    ) {
        match self {
            Self::NetQuake(storage) => storage.ring.observe_submission(sequence, observation),
            Self::QuakeWorld { ring, .. } => ring.observe_submission(sequence, observation),
            Self::Quake2(ring) => ring.observe_submission(sequence, observation),
            Self::Quake2Repro { ring, .. } => ring.observe_submission(sequence, observation),
            Self::Quake3(ring) => ring.observe_submission(sequence, observation),
        }
    }
    pub(crate) fn observation(
        &self,
        sequence: u32,
    ) -> Option<qa_core::primitives::SnapshotObservation> {
        match self {
            Self::NetQuake(storage) => storage.ring.observation(sequence, u32::MAX),
            Self::QuakeWorld { ring, .. } => ring.observation(sequence, u8::MAX.into()),
            Self::Quake2(ring) => ring.observation(sequence, u32::MAX),
            Self::Quake2Repro { ring, .. } => {
                ring.observation(sequence, Q2Header::REPRO_FRAME_MASK)
            }
            Self::Quake3(ring) => ring.observation(sequence, u32::MAX),
        }
    }
    pub(crate) fn load(
        protocol: packet::Protocol,
        endpoint: qa_core::loopback::Endpoint,
    ) -> Result<Self, packet::Error> {
        let limits = snapshot_limits(protocol);
        Ok(match protocol {
            packet::Protocol::NetQuake15 => Self::NetQuake(Box::new(NqStorage::load(endpoint)?)),
            packet::Protocol::QuakeWorld28 => Self::QuakeWorld {
                ring: Box::new(QwRing::load(
                    limits.entity_capacity
                        + if endpoint == qa_core::loopback::Endpoint::Client {
                            states::QW_PLAYER_SLOTS
                        } else {
                            0
                        },
                    limits.native_numbers,
                    limits.area_capacity,
                    limits.retained_rows,
                )?),
                player_commands: vec![
                    [packet::ZERO_QW; states::QW_PLAYER_SLOTS];
                    if endpoint == qa_core::loopback::Endpoint::Client {
                        64
                    } else {
                        0
                    }
                ]
                .into_boxed_slice(),
                player_model: 0,
            },
            packet::Protocol::Quake2_34 => Self::Quake2(Box::new(Q2Ring::load(
                limits.entity_capacity,
                limits.native_numbers,
                limits.area_capacity,
                limits.retained_rows,
            )?)),
            packet::Protocol::Quake2Repro1038 => Self::Quake2Repro {
                ring: Box::new(Q2ReproRing::load(
                    limits.entity_capacity,
                    limits.native_numbers,
                    limits.area_capacity,
                    limits.retained_rows,
                )?),
                frame_ms: 25,
                entity_policy: Q2EntityPolicy {
                    beam_old_origin_fix: true,
                    ..Default::default()
                },
            },
            packet::Protocol::Quake3_68 => Self::Quake3(Box::new(Q3Ring::load(
                limits.entity_capacity,
                limits.native_numbers,
                limits.area_capacity,
                limits.retained_rows,
            )?)),
        })
    }
    pub(crate) fn protocol(&self) -> packet::Protocol {
        match self {
            Self::NetQuake(_) => packet::Protocol::NetQuake15,
            Self::QuakeWorld { .. } => packet::Protocol::QuakeWorld28,
            Self::Quake2(_) => packet::Protocol::Quake2_34,
            Self::Quake2Repro { .. } => packet::Protocol::Quake2Repro1038,
            Self::Quake3(_) => packet::Protocol::Quake3_68,
        }
    }
    pub(crate) fn record_move(
        &mut self,
        sequence: u32,
        time: qa_core::sys_events::EventTime,
        command: crate::channel::MoveCommand,
    ) {
        match self {
            Self::NetQuake(storage) => storage.ring.record_move(sequence, time, command),
            Self::QuakeWorld { ring, .. } => ring.record_move(sequence, time, command),
            Self::Quake2(ring) => ring.record_move(sequence, time, command),
            Self::Quake2Repro { ring, .. } => ring.record_move(sequence, time, command),
            Self::Quake3(ring) => ring.record_move(sequence, time, command),
        }
    }
    pub(crate) fn submitted_command(&self, sequence: u32) -> Option<crate::channel::MoveCommand> {
        match self {
            Self::NetQuake(storage) => storage.ring.submitted_command(sequence),
            Self::QuakeWorld { ring, .. } => ring.submitted_command(sequence),
            Self::Quake2(ring) => ring.submitted_command(sequence),
            Self::Quake2Repro { ring, .. } => ring.submitted_command(sequence),
            Self::Quake3(ring) => ring.submitted_command(sequence),
        }
    }
    pub(crate) fn frame(&self, sequence: u32) -> Option<ReceivedFrame<'_>> {
        match self {
            Self::NetQuake(storage) => storage.ring.frame(sequence).map(ReceivedFrame::NetQuake),
            Self::QuakeWorld { ring, .. } => ring.frame(sequence).map(ReceivedFrame::QuakeWorld),
            Self::Quake2(ring) => ring.frame(sequence).map(ReceivedFrame::Quake2),
            Self::Quake2Repro { ring, .. } => ring.frame(sequence).map(ReceivedFrame::Quake2Repro),
            Self::Quake3(ring) => ring.frame(sequence).map(ReceivedFrame::Quake3),
        }
    }
    pub(crate) fn record_request(&mut self, sequence: u32, base: Option<u32>) {
        match self {
            Self::QuakeWorld { ring, .. } => ring.record_request(sequence, base),
            Self::NetQuake(_) | Self::Quake2(_) | Self::Quake2Repro { .. } | Self::Quake3(_) => {}
        }
    }
    pub(crate) fn current(&self) -> Option<ReceivedFrame<'_>> {
        let sequence = match self {
            Self::NetQuake(storage) => storage.ring.latest,
            Self::QuakeWorld { ring, .. } => ring.latest,
            Self::Quake2(ring) => ring.latest,
            Self::Quake2Repro { ring, .. } => ring.latest,
            Self::Quake3(ring) => ring.latest,
        }?;
        self.frame(sequence)
    }
    pub(crate) fn previous(&self) -> Option<ReceivedFrame<'_>> {
        match self {
            Self::NetQuake(storage) => storage.ring.previous().map(ReceivedFrame::NetQuake),
            Self::QuakeWorld { ring, .. } => ring.previous().map(ReceivedFrame::QuakeWorld),
            Self::Quake2(ring) => ring.previous().map(ReceivedFrame::Quake2),
            Self::Quake2Repro { ring, .. } => ring.previous().map(ReceivedFrame::Quake2Repro),
            Self::Quake3(ring) => ring.previous().map(ReceivedFrame::Quake3),
        }
    }
    pub(crate) fn store(&mut self, frame: ReceivedFrame<'_>) -> Result<(), packet::Error> {
        match (self, frame) {
            (Self::NetQuake(storage), ReceivedFrame::NetQuake(frame)) => storage.ring.store(frame),
            (Self::QuakeWorld { ring, .. }, ReceivedFrame::QuakeWorld(frame)) => ring.store(frame),
            (Self::Quake2(ring), ReceivedFrame::Quake2(frame)) => ring.store(frame),
            (Self::Quake2Repro { ring, .. }, ReceivedFrame::Quake2Repro(frame)) => {
                ring.store(frame)
            }
            (Self::Quake3(ring), ReceivedFrame::Quake3(frame)) => ring.store(frame),
            _ => Err(packet::Error::Context),
        }
    }
    pub(crate) fn write_candidate(
        &self,
        writer: &mut Writer<'_>,
        frame: ReceivedFrame<'_>,
        request: Option<u32>,
        native_clients: u32,
    ) -> Result<(), packet::Error> {
        match (self, frame) {
            (Self::NetQuake(storage), ReceivedFrame::NetQuake(frame)) => {
                write_nq_frame(writer, &storage.ring, frame)
            }
            (Self::QuakeWorld { ring, .. }, ReceivedFrame::QuakeWorld(frame)) => write_qw_frame(
                writer,
                ring,
                frame,
                request.and_then(|request| {
                    let base = ring.slots[slot(request)].sequence?;
                    (base & 63 == request & 63 && ring.frame(base).is_some())
                        .then_some((base, request as u8))
                }),
            ),
            (Self::Quake2(ring), ReceivedFrame::Quake2(frame)) => {
                write_q2_frame(writer, ring, frame, request, native_clients)
            }
            (
                Self::Quake2Repro {
                    ring,
                    entity_policy,
                    ..
                },
                ReceivedFrame::Quake2Repro(frame),
            ) => {
                let mut policy = *entity_policy;
                policy.native_clients = native_clients;
                q2::write_q2_repro_frame(writer, ring, frame, request, policy)
            }
            (Self::Quake3(ring), ReceivedFrame::Quake3(frame)) => {
                write_q3_frame(writer, ring, frame, request)
            }
            _ => Err(packet::Error::Context),
        }
    }
    pub(crate) fn normalize_written(
        &mut self,
        sequence: u32,
        request: Option<u32>,
    ) -> Result<(), packet::Error> {
        if let Self::Quake2Repro {
            ring,
            entity_policy,
            ..
        } = self
        {
            q2::prepare_first_person(ring, sequence, request, entity_policy.first_person)?;
        }
        Ok(())
    }
    pub(crate) fn write(
        &mut self,
        writer: &mut Writer<'_>,
        sequence: u32,
        request: Option<u32>,
        native_clients: u32,
    ) -> Result<(), packet::Error> {
        self.normalize_written(sequence, request)?;
        let frame = self.frame(sequence).ok_or(packet::Error::Context)?;
        self.write_candidate(writer, frame, request, native_clients)
    }
}

/// Original svc_snapshot body after its opcode. Missing deltas are consumed
/// without publishing, so following server commands remain in the same stream.
pub fn read_q3(
    reader: &mut Reader<'_>,
    ring: &mut Q3Ring,
    sequence: u32,
    command: u32,
) -> Result<bool, packet::Error> {
    let time =
        qa_core::primitives::ThinkTime::Milliseconds(i64::from(reader.read_bits(32)? as i32));
    let distance = reader.read_bits(8)?;
    let base_sequence = sequence.saturating_sub(distance);
    let full = distance == 0 || base_sequence == 0;
    let flags = reader.read_bits(8)? as u8;
    let area_bytes = read_areas(reader, &mut ring.scratch_areas, 32, false)?;
    let base_index = slot(base_sequence);
    let base_valid = full || ring.frame(base_sequence).is_some();
    // qsrc consumes an invalid delta using the addressed slot's retained
    // state too; those discarded rows still advance native retention age.
    let old = if !full {
        ring.slots[base_index]
    } else {
        Slot::ZERO
    };
    let player = states::read_q3_player(reader, &old.player)?;
    let (count, overflow) = read_entities(
        reader,
        rows(&ring.entities, ring.capacity, base_index, old.count),
        &ring.baselines,
        &mut ring.scratch,
        MergeRules {
            terminator: 1023,
            remove_advances_old: false,
        },
        |_| false,
        |reader| {
            Ok(states::EntityHeader {
                number: reader.read_bits(10)? as u16,
                flags: 0,
            })
        },
        |reader, header, from| Ok(states::read_q3_entity_body(reader, header.number, from)?.words),
        |from| *from,
    )?;
    Ok(ring.publish_received(
        Slot {
            sequence: Some(sequence),
            valid: false,
            time,
            command,
            flags,
            area_bytes,
            count,
            first_entity: 0,
            player,
            ..Slot::ZERO
        },
        base_valid,
        overflow,
        false,
    ))
}

/// Protocol-34 svc_frame body. Unlike Q3, its frame number is carried in the
/// payload and its native server time is frame * 100 milliseconds.
pub fn read_q2(reader: &mut Reader<'_>, ring: &mut Q2Ring) -> Result<bool, packet::Error> {
    q2::read_records::<false, { states::Q2_PLAYER_WORDS }, { states::Q2_ENTITY_WORDS }>(
        reader,
        ring,
        q2::ReadRules {
            area_limit: 32,
            entity_limit: 1024,
            entity_opcode: true,
            extended_header: false,
            valid_base: false,
        },
        |reader, from, _| states::read_q2_player(reader, from),
        |reader, header, from| Ok(states::read_q2_entity_body(reader, header, from)?.words),
        |from| states::q2_unchanged_entity(from, false),
        |sequence| {
            qa_core::primitives::ThinkTime::Milliseconds(i64::from(
                (sequence as i32).wrapping_mul(100),
            ))
        },
    )
}

/// Body after svc_packetentities (delta=false) or svc_deltapacketentities.
/// qsrc selects the base from the request associated with this incoming frame;
/// its low-byte wire prefix merely warns on mismatch and never selects a base.
pub fn read_qw(
    reader: &mut Reader<'_>,
    ring: &mut QwRing,
    sequence: u32,
    delta: bool,
    requested_base: Option<u32>,
    outgoing_sequence: u32,
) -> Result<bool, packet::Error> {
    read_qw_records(
        reader,
        ring,
        sequence,
        delta,
        requested_base,
        outgoing_sequence,
        None,
    )
}
pub(crate) fn read_qw_players(
    reader: &mut Reader<'_>,
    ring: &mut QwRing,
    sequence: u32,
    delta: bool,
    requested_base: Option<u32>,
    outgoing_sequence: u32,
    players: &[Option<states::QwPlayerInfo>; states::QW_PLAYER_SLOTS],
    projection: &crate::projection::PlayerProjection,
    namespace: qa_core::primitives::ResourceNamespace,
) -> Result<bool, packet::Error> {
    read_qw_records(
        reader,
        ring,
        sequence,
        delta,
        requested_base,
        outgoing_sequence,
        Some((players, projection, namespace)),
    )
}
#[expect(
    clippy::too_many_arguments,
    reason = "Native player records share the existing snapshot merge and load-built projector"
)]
fn read_qw_records(
    reader: &mut Reader<'_>,
    ring: &mut QwRing,
    sequence: u32,
    delta: bool,
    requested_base: Option<u32>,
    outgoing_sequence: u32,
    players: Option<(
        &[Option<states::QwPlayerInfo>; states::QW_PLAYER_SLOTS],
        &crate::projection::PlayerProjection,
        qa_core::primitives::ResourceNamespace,
    )>,
) -> Result<bool, packet::Error> {
    if delta {
        reader.read_bits(8)?;
    }
    let request = if delta { requested_base } else { None };
    let full = !delta;
    let base_valid = full
        || request
            .is_some_and(|n| outgoing_sequence.wrapping_sub(n) < 63 && ring.frame(n).is_some());
    let index = request.map_or(0, slot);
    let count = if request.is_none() {
        0
    } else {
        ring.slots[index].count
    };
    let mut invalid_full = false;
    let old = rows(&ring.entities, ring.capacity, index, count);
    let previous_peers = if players.is_some() {
        ring.slots[index].qw_peers
    } else {
        0
    };
    let (mut count, mut overflow) = read_entities(
        reader,
        old,
        &ring.baselines,
        &mut ring.scratch,
        MergeRules {
            terminator: 0,
            remove_advances_old: false,
        },
        |row| {
            row.number.checked_sub(1).is_some_and(|n| {
                n < states::QW_PLAYER_SLOTS as u32 && previous_peers & (1 << n) != 0
            })
        },
        |reader| {
            let mut header = states::read_qw_entity_header(reader)?;
            // CL_ParsePacketEntities casts MSG_ReadShort to unsigned short
            // before CL_ParseDelta, unlike the standalone native scalar reader.
            header.flags &= 0xffff;
            if header.number == 0 && header.flags != 0 {
                return Err(packet::Error::Count);
            }
            invalid_full |= full && header.flags & (1 << 14) != 0;
            Ok(header)
        },
        |reader, header, from| Ok(states::read_qw_entity_body(reader, header, from)?.words),
        |from| *from,
    )?;
    if invalid_full {
        return Err(packet::Error::Opcode);
    }
    overflow |= count > 64;
    let mut peer_mask = 0;
    if !overflow
        && base_valid
        && let Some((players, projection, namespace)) = players
    {
        let peers = players.iter().flatten().count();
        if count + peers > ring.scratch.len() {
            overflow = true;
        } else {
            let total = count + peers;
            let mut out = total;
            let entity_projection =
                crate::projection::EntityProjection::load(packet::Protocol::QuakeWorld28);
            let mut infos = players.iter().flatten().rev().peekable();
            while count != 0 || infos.peek().is_some() {
                let peer_number = infos.peek().map(|info| u32::from(info.number) + 1);
                let packet = if count != 0 {
                    Some(ring.scratch[count - 1])
                } else {
                    None
                };
                let row = if let Some(info) = infos.peek()
                    && packet
                        .is_none_or(|row| peer_number.is_some_and(|number| number >= row.number))
                {
                    let info = **info;
                    infos.next();
                    if packet.is_some_and(|row| peer_number == Some(row.number)) {
                        count -= 1;
                    }
                    let mut player = qa_core::primitives::PlayerState::default();
                    let mut context = crate::projection::PlayerContext::default();
                    projection.apply(&info.words, &mut player, &mut context, namespace, |_| None);
                    let mut record = crate::projection::EntityRecord {
                        position: player.body.position,
                        velocity: player.body.velocity,
                        angles: info.command.view_angles,
                        appearance: context.appearance.ok_or(packet::Error::Context)?,
                    };
                    record.appearance.native.colormap = u32::from(info.number) + 1;
                    let mut words = [0; states::QW_ENTITY_WORDS];
                    entity_projection.reduce(
                        &record,
                        &mut words,
                        |model| Some(model.ordinal),
                        |sound| Some(sound.ordinal),
                        |reference| Some(reference.ordinal()),
                    );
                    peer_mask |= 1 << info.number;
                    Entity {
                        number: u32::from(info.number) + 1,
                        words,
                    }
                } else {
                    count -= 1;
                    ring.scratch[count]
                };
                out -= 1;
                ring.scratch[out] = row;
            }
            ring.scratch.copy_within(out..total, 0);
            count = total - out;
        }
    }
    Ok(ring.publish_received(
        Slot {
            sequence: Some(sequence),
            count,
            qw_peers: peer_mask,
            ..Slot::ZERO
        },
        base_valid,
        overflow,
        true,
    ))
}

// The prefix and unchanged-row policy are native format data. The ordered
// entity/baseline/removal merge is shared by every packet-frame decoder.
struct MergeRules {
    terminator: u16,
    remove_advances_old: bool,
}

#[expect(
    clippy::too_many_arguments,
    reason = "Typed native codec entries keep the one hot merge monomorphized"
)]
fn read_entities<const E: usize>(
    reader: &mut Reader<'_>,
    old: &[Entity<E>],
    baselines: &[[u32; E]],
    scratch: &mut [Entity<E>],
    rules: MergeRules,
    skip_old: impl Fn(&Entity<E>) -> bool,
    mut header: impl FnMut(&mut Reader<'_>) -> Result<states::EntityHeader, packet::Error>,
    mut body: impl FnMut(
        &mut Reader<'_>,
        states::EntityHeader,
        &[u32; E],
    ) -> Result<Option<[u32; E]>, packet::Error>,
    unchanged: impl Fn(&[u32; E]) -> [u32; E],
) -> Result<(usize, bool), packet::Error> {
    let mut old = old.iter().filter(|row| !skip_old(row)).peekable();
    let mut count = 0;
    let mut overflow = false;
    let mut previous = None;
    loop {
        let prefix = header(reader)?;
        let number = prefix.number;
        if number == rules.terminator {
            break;
        }
        if previous.is_some_and(|n| number <= n) {
            return Err(packet::Error::Count);
        }
        previous = Some(number);
        while let Some(row) = old.peek() {
            if row.number >= u32::from(number) {
                break;
            }
            let mut entity = **row;
            entity.words = unchanged(&entity.words);
            append(scratch, entity, &mut count, &mut overflow);
            old.next();
        }
        let matched = old
            .peek()
            .filter(|row| row.number == u32::from(number))
            .map(|row| **row);
        let from = if let Some(entity) = matched {
            old.next();
            entity.words
        } else {
            *baselines.get(number as usize).ok_or(packet::Error::Count)?
        };
        if let Some(words) = body(reader, prefix, &from)? {
            append(
                scratch,
                Entity {
                    number: u32::from(number),
                    words,
                },
                &mut count,
                &mut overflow,
            );
        } else if rules.remove_advances_old && matched.is_none() {
            old.next();
        }
    }
    for row in old {
        let mut entity = *row;
        entity.words = unchanged(&entity.words);
        append(scratch, entity, &mut count, &mut overflow);
    }
    Ok((count, overflow))
}

fn append<const E: usize>(
    out: &mut [Entity<E>],
    entity: Entity<E>,
    count: &mut usize,
    overflow: &mut bool,
) {
    if let Some(slot) = out.get_mut(*count) {
        *slot = entity;
    } else {
        *overflow = true;
    }
    *count += 1;
}

fn read_areas(
    reader: &mut Reader<'_>,
    out: &mut [u8],
    limit: usize,
    visible: bool,
) -> Result<usize, packet::Error> {
    let count = reader.read_bits(8)? as usize;
    if count > out.len() || count > limit {
        return Err(packet::Error::Count);
    }
    reader.read_data(&mut out[..count])?;
    if visible {
        for byte in &mut out[..count] {
            *byte = native_area_byte(*byte, visible);
        }
    }
    Ok(count)
}

fn delta_frame<const P: usize, const E: usize>(
    ring: &Ring<P, E>,
    sequence: u32,
    request: Option<u32>,
) -> Option<Frame<'_, P, E>> {
    request
        .filter(|n| *n > 0 && sequence.checked_sub(*n).is_some_and(|d| d < 29))
        .and_then(|n| ring.frame(n))
}

fn native_entities<const E: usize>(entities: &[Entity<E>], first: u32, end: u32) -> &[Entity<E>] {
    let a = entities.partition_point(|e| e.number < first);
    let b = entities.partition_point(|e| e.number < end);
    &entities[a..b]
}

/// Native SV_WriteSnapshotToClient/SV_EmitPacketEntities ordering. The caller
/// supplies the client's actual delta request; no transmit watermark is an ACK.
pub fn write_q3(
    writer: &mut Writer<'_>,
    ring: &Q3Ring,
    sequence: u32,
    delta_request: Option<u32>,
) -> Result<(), packet::Error> {
    let frame = ring.frame(sequence).ok_or(packet::Error::Context)?;
    write_q3_frame(writer, ring, frame, delta_request)
}
pub(super) fn write_q3_frame(
    writer: &mut Writer<'_>,
    ring: &Q3Ring,
    to: Frame<'_, { states::PLAYER_WORDS }, { states::ENTITY_WORDS }>,
    delta_request: Option<u32>,
) -> Result<(), packet::Error> {
    let sequence = to.sequence;
    let areas = &to.areas[..to.areas.len().min(32)];
    // 1023 is the native packet-entity terminator. Higher common/native
    // namespace entries cannot be represented by this negotiated protocol.
    let entities = native_entities(to.entities, 0, 1023);
    let from = delta_frame(ring, sequence, delta_request);
    writer.write_bits(7, 8)?;
    let time = to.time.milliseconds() as u32;
    writer.write_bits(time, 32)?;
    writer.write_bits(from.map_or(0, |f| sequence - f.sequence), 8)?;
    writer.write_bits(u32::from(to.flags), 8)?;
    write_areas(writer, areas, false)?;
    states::write_q3_player(
        writer,
        from.map_or(&[0; states::PLAYER_WORDS], |f| f.player),
        to.player,
    )?;
    let old = from.map_or(&[][..], |f| native_entities(f.entities, 0, 1023));
    write_entities(
        writer,
        old,
        entities,
        &ring.baselines,
        states::write_q3_entity,
    )?;
    writer.write_bits(1023, 10)?;
    Ok(())
}

/// Native protocol-34 frame and packet entities. The caller supplies the
/// acknowledged lastframe and its native client count, not common slot ids.
pub fn write_q2(
    writer: &mut Writer<'_>,
    ring: &Q2Ring,
    sequence: u32,
    delta_request: Option<u32>,
    native_clients: u32,
) -> Result<(), packet::Error> {
    let frame = ring.frame(sequence).ok_or(packet::Error::Context)?;
    write_q2_frame(writer, ring, frame, delta_request, native_clients)
}
pub(super) fn write_q2_frame(
    writer: &mut Writer<'_>,
    ring: &Q2Ring,
    to: Frame<'_, { states::Q2_PLAYER_WORDS }, { states::Q2_ENTITY_WORDS }>,
    delta_request: Option<u32>,
    native_clients: u32,
) -> Result<(), packet::Error> {
    q2::write_records::<false, { states::Q2_PLAYER_WORDS }, { states::Q2_ENTITY_WORDS }>(
        writer,
        ring,
        to,
        delta_request,
        q2::WriteRules {
            area_limit: 32,
            entity_limit: 1024,
            entity_opcode: true,
        },
        |writer, from, to| states::write_q2_player(writer, from, to).map(|()| 0),
        |writer, number, old, new, force| {
            states::write_q2_entity(
                writer,
                number,
                old,
                new,
                force,
                force || number <= native_clients,
            )
        },
    )
}

/// Native protocol-28 packetentities. An unavailable retained request receives
/// a native full update; this does not add an extension to the wire.
pub fn write_qw(
    writer: &mut Writer<'_>,
    ring: &QwRing,
    sequence: u32,
    delta_request: Option<(u32, u8)>,
) -> Result<(), packet::Error> {
    let frame = ring.frame(sequence).ok_or(packet::Error::Context)?;
    write_qw_frame(writer, ring, frame, delta_request)
}
pub(super) fn write_qw_frame(
    writer: &mut Writer<'_>,
    ring: &QwRing,
    to: Frame<'_, 0, { states::QW_ENTITY_WORDS }>,
    delta_request: Option<(u32, u8)>,
) -> Result<(), packet::Error> {
    let from = delta_request.and_then(|(n, byte)| ring.frame(n).map(|frame| (frame, byte)));
    writer.write_bits(if from.is_some() { 48 } else { 47 }, 8)?;
    if let Some((_, byte)) = from {
        writer.write_bits(u32::from(byte), 8)?;
    }
    let entities = native_entities(to.entities, 1, 512);
    let entities = &entities[..entities.len().min(64)];
    let old = from.map_or(&[][..], |(f, _)| native_entities(f.entities, 1, 512));
    let old = &old[..old.len().min(64)];
    write_entities(
        writer,
        old,
        entities,
        &ring.baselines,
        states::write_qw_entity,
    )?;
    writer.write_bits(0, 16)?;
    Ok(())
}

pub fn write_nq(
    writer: &mut Writer<'_>,
    ring: &NqRing,
    sequence: u32,
) -> Result<(), packet::Error> {
    let frame = ring.frame(sequence).ok_or(packet::Error::Context)?;
    write_nq_frame(writer, ring, frame)
}
pub(super) fn write_nq_frame(
    writer: &mut Writer<'_>,
    ring: &NqRing,
    to: Frame<'_, { states::NQ_PLAYER_WORDS }, { states::NQ_ENTITY_WORDS }>,
) -> Result<(), packet::Error> {
    let seconds = to.time.seconds() as f32;
    writer.write_bits(7, 8)?;
    writer.write_bits(seconds.to_bits(), 32)?;
    states::write_nq_player(writer, to.player)?;
    for entity in native_entities(to.entities, 1, 600) {
        states::write_nq_entity(
            writer,
            entity.number,
            &ring.baselines[entity.number as usize],
            &entity.words,
            entity.words[11] != 0,
        )?;
    }
    Ok(())
}

fn write_entities<const E: usize>(
    writer: &mut Writer<'_>,
    old: &[Entity<E>],
    entities: &[Entity<E>],
    baselines: &[[u32; E]],
    mut encode: impl FnMut(
        &mut Writer<'_>,
        u32,
        &[u32; E],
        Option<&[u32; E]>,
        bool,
    ) -> Result<bool, crate::message::Error>,
) -> Result<(), packet::Error> {
    let mut a = 0;
    let mut b = 0;
    while a < old.len() || b < entities.len() {
        let old_number = old.get(a).map_or(u32::MAX, |e| e.number);
        let new_number = entities.get(b).map_or(u32::MAX, |e| e.number);
        if old_number == new_number {
            encode(
                writer,
                new_number,
                &old[a].words,
                Some(&entities[b].words),
                false,
            )?;
            a += 1;
            b += 1;
        } else if new_number < old_number {
            encode(
                writer,
                new_number,
                &baselines[new_number as usize],
                Some(&entities[b].words),
                true,
            )?;
            b += 1;
        } else {
            encode(writer, old_number, &old[a].words, None, true)?;
            a += 1;
        }
    }
    Ok(())
}

fn rows<T>(items: &[T], capacity: usize, index: usize, count: usize) -> &[T] {
    let first = index * capacity;
    &items[first..first + count]
}
fn rows_mut<T>(items: &mut [T], capacity: usize, index: usize, count: usize) -> &mut [T] {
    let first = index * capacity;
    &mut items[first..first + count]
}
fn write_areas(
    writer: &mut Writer<'_>,
    areas: &[u8],
    visible: bool,
) -> Result<(), crate::message::Error> {
    writer.write_bits(areas.len() as u32, 8)?;
    writer.write_data_iter(areas.iter().map(|byte| native_area_byte(*byte, visible)))
}
