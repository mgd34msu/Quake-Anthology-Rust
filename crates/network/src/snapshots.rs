//! Connection-owned native projections. Storage is reserved at connect;
//! snapshot retention never owns another engine player/entity implementation.
use crate::{
    commands::packet,
    message::{Reader, Writer},
    states,
};
mod q2;
pub use q2::{Q2Header, Q2KexContext, Q2KexFrame, Q2KexRing, read_q2_kex};

pub const SLOTS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entity<const N: usize> {
    pub number: u32,
    pub words: [u32; N],
}

#[derive(Clone, Copy)]
struct Slot<const P: usize> {
    sequence: Option<u32>,
    request_sequence: Option<u32>,
    requested_base: Option<u32>,
    valid: bool,
    time: qa_core::primitives::ThinkTime,
    command: u32,
    flags: u8,
    area_bytes: usize,
    count: usize,
    first_entity: u64,
    player: [u32; P],
}
impl<const P: usize> Slot<P> {
    const ZERO: Self = Self {
        sequence: None,
        request_sequence: None,
        requested_base: None,
        valid: false,
        time: qa_core::primitives::ThinkTime::Milliseconds(0),
        command: 0,
        flags: 0,
        area_bytes: 0,
        count: 0,
        first_entity: 0,
        player: [0; P],
    };
}

#[derive(Clone, Copy, Debug)]
pub struct Frame<'a, const P: usize, const E: usize> {
    pub sequence: u32,
    pub time: qa_core::primitives::ThinkTime,
    pub command: u32,
    pub flags: u8,
    pub areas: &'a [u8],
    pub player: &'a [u32; P],
    pub entities: &'a [Entity<E>],
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
    counts: Counts,
}

impl<const P: usize, const E: usize> Ring<P, E> {
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
            counts: Counts::default(),
        })
    }
    pub fn counts(&self) -> Counts {
        self.counts
    }
    /// Association for the native reply to a submitted client packet. These
    /// scalars are protocol metadata, never retained input or another frame.
    pub(crate) fn record_request(&mut self, sequence: u32, base: Option<u32>) {
        let slot = &mut self.slots[sequence as usize & (SLOTS - 1)];
        slot.request_sequence = Some(sequence);
        slot.requested_base = base;
    }
    pub(crate) fn requested_base(&self, sequence: u32) -> Option<Option<u32>> {
        let slot = &self.slots[sequence as usize & (SLOTS - 1)];
        (slot.request_sequence == Some(sequence)).then_some(slot.requested_base)
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
    pub fn frame(&self, sequence: u32) -> Option<Frame<'_, P, E>> {
        let index = sequence as usize & (SLOTS - 1);
        let slot = &self.slots[index];
        if !slot.valid
            || slot.sequence != Some(sequence)
            || self
                .retained_rows
                .is_some_and(|n| self.parsed_rows.saturating_sub(slot.first_entity) > n)
        {
            return None;
        }
        Some(Frame {
            sequence,
            time: slot.time,
            command: slot.command,
            flags: slot.flags,
            player: &slot.player,
            areas: &self.areas
                [index * self.area_capacity..index * self.area_capacity + slot.area_bytes],
            entities: &self.entities[index * self.capacity..index * self.capacity + slot.count],
        })
    }
    /// An invalid Q2 current frame requests a full update, even when an older
    /// accepted frame remains available as a future native delta base.
    pub fn current(&self) -> Option<Frame<'_, P, E>> {
        self.frame(self.latest?)
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
            area_bytes: frame.areas.len(),
            count: frame.entities.len(),
            first_entity,
            player: *frame.player,
            ..Slot::ZERO
        };
        let index = frame.sequence as usize & (SLOTS - 1);
        self.areas[index * self.area_capacity..index * self.area_capacity + frame.areas.len()]
            .copy_from_slice(frame.areas);
        self.entities[index * self.capacity..index * self.capacity + frame.entities.len()]
            .copy_from_slice(frame.entities);
        self.commit(slot);
        Ok(())
    }
    fn commit(&mut self, mut slot: Slot<P>) {
        let Some(sequence) = slot.sequence else {
            return;
        };
        let start = self
            .latest
            .map_or(1, |n| n.saturating_add(1))
            .max(sequence.saturating_sub((SLOTS - 1) as u32));
        for missing in start..sequence {
            self.slots[missing as usize & (SLOTS - 1)].sequence = None;
        }
        let index = sequence as usize & (SLOTS - 1);
        slot.request_sequence = self.slots[index].request_sequence;
        slot.requested_base = self.slots[index].requested_base;
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
        let index = sequence as usize & (SLOTS - 1);
        self.areas[index * self.area_capacity..index * self.area_capacity + slot.area_bytes]
            .copy_from_slice(&self.scratch_areas[..slot.area_bytes]);
        self.entities[index * self.capacity..index * self.capacity + slot.count]
            .copy_from_slice(&self.scratch[..slot.count]);
        slot.valid = base_valid;
        self.commit(slot);
        base_valid
    }
}

pub type Q3Ring = Ring<{ states::PLAYER_WORDS }, { states::ENTITY_WORDS }>;
pub type Q3Frame<'a> = Frame<'a, { states::PLAYER_WORDS }, { states::ENTITY_WORDS }>;
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
        let scratch = if endpoint == qa_core::loopback::Endpoint::Client {
            600
        } else {
            0
        };
        Ok(Self {
            ring: NqRing::load(600, 600, 0, None)?,
            weapon_is_mask: false,
            marks: qa_core::stamps::StampSet::new(scratch),
            indices: vec![0; scratch].into_boxed_slice(),
            times: vec![0.0; scratch].into_boxed_slice(),
        })
    }
    pub fn begin(&mut self) -> NqPacket {
        self.marks.begin();
        let old = self
            .ring
            .latest
            .map(|n| self.ring.slots[n as usize & (SLOTS - 1)]);
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
            let start = old.sequence.map_or(0, |n| n as usize & (SLOTS - 1)) * self.ring.capacity;
            self.ring.scratch[..old.count]
                .copy_from_slice(&self.ring.entities[start..start + old.count]);
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
    Quake3(Q3Frame<'a>),
}
impl ReceivedFrame<'_> {
    pub fn time(self) -> qa_core::primitives::ThinkTime {
        match self {
            Self::NetQuake(frame) => frame.time,
            Self::QuakeWorld(frame) => frame.time,
            Self::Quake2(frame) => frame.time,
            Self::Quake3(frame) => frame.time,
        }
    }
    pub fn sequence(self) -> u32 {
        match self {
            Self::NetQuake(frame) => frame.sequence,
            Self::QuakeWorld(frame) => frame.sequence,
            Self::Quake2(frame) => frame.sequence,
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
        player_commands: Box<[[crate::commands::QwCmd; 32]]>,
        player_model: u32,
    },
    Quake2(Box<Q2Ring>),
    Quake3(Box<Q3Ring>),
}
impl Storage {
    pub(crate) fn load(
        protocol: packet::Protocol,
        endpoint: qa_core::loopback::Endpoint,
    ) -> Result<Self, packet::Error> {
        Ok(match protocol {
            packet::Protocol::NetQuake15 => Self::NetQuake(Box::new(NqStorage::load(endpoint)?)),
            packet::Protocol::QuakeWorld28 => Self::QuakeWorld {
                ring: Box::new(QwRing::load(64, 512, 0, None)?),
                player_commands: vec![
                    [packet::ZERO_QW; 32];
                    if endpoint == qa_core::loopback::Endpoint::Client {
                        64
                    } else {
                        0
                    }
                ]
                .into_boxed_slice(),
                player_model: 0,
            },
            packet::Protocol::Quake2_34 => {
                Self::Quake2(Box::new(Q2Ring::load(1023, 1024, 32, Some(1024 - 128))?))
            }
            packet::Protocol::Quake3_68 => {
                Self::Quake3(Box::new(Q3Ring::load(1023, 1024, 32, Some(2048 - 128))?))
            }
        })
    }
    pub(crate) fn protocol(&self) -> packet::Protocol {
        match self {
            Self::NetQuake(_) => packet::Protocol::NetQuake15,
            Self::QuakeWorld { .. } => packet::Protocol::QuakeWorld28,
            Self::Quake2(_) => packet::Protocol::Quake2_34,
            Self::Quake3(_) => packet::Protocol::Quake3_68,
        }
    }
    pub(crate) fn frame(&self, sequence: u32) -> Option<ReceivedFrame<'_>> {
        match self {
            Self::NetQuake(storage) => storage.ring.frame(sequence).map(ReceivedFrame::NetQuake),
            Self::QuakeWorld { ring, .. } => ring.frame(sequence).map(ReceivedFrame::QuakeWorld),
            Self::Quake2(ring) => ring.frame(sequence).map(ReceivedFrame::Quake2),
            Self::Quake3(ring) => ring.frame(sequence).map(ReceivedFrame::Quake3),
        }
    }
    pub(crate) fn record_request(&mut self, sequence: u32, base: Option<u32>) {
        match self {
            Self::QuakeWorld { ring, .. } => ring.record_request(sequence, base),
            Self::NetQuake(_) | Self::Quake2(_) | Self::Quake3(_) => {}
        }
    }
    pub(crate) fn current(&self) -> Option<ReceivedFrame<'_>> {
        let sequence = match self {
            Self::NetQuake(storage) => storage.ring.latest,
            Self::QuakeWorld { ring, .. } => ring.latest,
            Self::Quake2(ring) => ring.latest,
            Self::Quake3(ring) => ring.latest,
        }?;
        self.frame(sequence)
    }
    pub(crate) fn store(&mut self, frame: ReceivedFrame<'_>) -> Result<(), packet::Error> {
        match (self, frame) {
            (Self::NetQuake(storage), ReceivedFrame::NetQuake(frame)) => storage.ring.store(frame),
            (Self::QuakeWorld { ring, .. }, ReceivedFrame::QuakeWorld(frame)) => ring.store(frame),
            (Self::Quake2(ring), ReceivedFrame::Quake2(frame)) => ring.store(frame),
            (Self::Quake3(ring), ReceivedFrame::Quake3(frame)) => ring.store(frame),
            _ => Err(packet::Error::Context),
        }
    }
    pub(crate) fn write(
        &self,
        writer: &mut Writer<'_>,
        sequence: u32,
        request: Option<u32>,
        native_clients: u32,
    ) -> Result<(), packet::Error> {
        match self {
            Self::NetQuake(storage) => write_nq(writer, &storage.ring, sequence),
            Self::QuakeWorld { ring, .. } => {
                // SV_EmitPacketEntities selects delta_sequence & UPDATE_MASK,
                // not an assumed full frame number made from the request byte.
                let request = request.and_then(|request| {
                    let slot = ring.slots[request as usize & (SLOTS - 1)];
                    let base = slot.sequence?;
                    (base & 63 == request & 63 && ring.frame(base).is_some())
                        .then_some((base, request as u8))
                });
                write_qw(writer, ring, sequence, request)
            }
            Self::Quake2(ring) => write_q2(writer, ring, sequence, request, native_clients),
            Self::Quake3(ring) => write_q3(writer, ring, sequence, request),
        }
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
    let area_bytes = read_areas(reader, &mut ring.scratch_areas, 32)?;
    let base_index = base_sequence as usize & (SLOTS - 1);
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
        &ring.entities[base_index * ring.capacity..base_index * ring.capacity + old.count],
        &ring.baselines,
        &mut ring.scratch,
        MergeRules {
            terminator: 1023,
            remove_advances_old: false,
        },
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
    if delta {
        reader.read_bits(8)?;
    }
    let request = if delta { requested_base } else { None };
    let full = !delta;
    let base_valid = full
        || request
            .is_some_and(|n| outgoing_sequence.wrapping_sub(n) < 63 && ring.frame(n).is_some());
    let index = request.map_or(0, |n| n as usize & (SLOTS - 1));
    let count = if request.is_none() {
        0
    } else {
        ring.slots[index].count
    };
    let mut invalid_full = false;
    let (count, overflow) = read_entities(
        reader,
        &ring.entities[index * ring.capacity..index * ring.capacity + count],
        &ring.baselines,
        &mut ring.scratch,
        MergeRules {
            terminator: 0,
            remove_advances_old: false,
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
    Ok(ring.publish_received(
        Slot {
            sequence: Some(sequence),
            count,
            ..Slot::ZERO
        },
        base_valid,
        overflow || count > 64,
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
    mut header: impl FnMut(&mut Reader<'_>) -> Result<states::EntityHeader, packet::Error>,
    mut body: impl FnMut(
        &mut Reader<'_>,
        states::EntityHeader,
        &[u32; E],
    ) -> Result<Option<[u32; E]>, packet::Error>,
    unchanged: impl Fn(&[u32; E]) -> [u32; E],
) -> Result<(usize, bool), packet::Error> {
    let mut old_index = 0;
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
        while old_index < old.len() {
            let mut entity = old[old_index];
            if entity.number >= u32::from(number) {
                break;
            }
            entity.words = unchanged(&entity.words);
            append(scratch, entity, &mut count, &mut overflow);
            old_index += 1;
        }
        let matched = old.get(old_index).filter(|e| e.number == u32::from(number));
        let from = if let Some(e) = matched {
            old_index += 1;
            e.words
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
            // Native Q2 advances the old cursor on every U_REMOVE, including
            // an unmatched number; QW/Q3 keep their own removal behaviour.
            old_index += 1;
        }
    }
    while old_index < old.len() {
        let mut entity = old[old_index];
        entity.words = unchanged(&entity.words);
        append(scratch, entity, &mut count, &mut overflow);
        old_index += 1;
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
) -> Result<usize, packet::Error> {
    let count = reader.read_bits(8)? as usize;
    if count > out.len() || count > limit {
        return Err(packet::Error::Count);
    }
    reader.read_data(&mut out[..count])?;
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
    let to = ring.frame(sequence).ok_or(packet::Error::Context)?;
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
    writer.write_bits(areas.len() as u32, 8)?;
    for &byte in areas {
        writer.write_bits(u32::from(byte), 8)?;
    }
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
    let to = ring.frame(sequence).ok_or(packet::Error::Context)?;
    let from = delta_frame(ring, sequence, delta_request);
    let areas = &to.areas[..to.areas.len().min(32)];
    Q2Header {
        sequence,
        delta: from.map_or(-1, |f| f.sequence as i32),
        flags: to.flags,
        player_flags: 0,
    }
    .write::<false>(writer, areas)?;
    states::write_q2_player(
        writer,
        from.map_or(&[0; states::Q2_PLAYER_WORDS], |f| f.player),
        to.player,
    )?;
    writer.write_bits(18, 8)?;
    write_entities(
        writer,
        from.map_or(&[][..], |f| native_entities(f.entities, 1, 1024)),
        native_entities(to.entities, 1, 1024),
        &ring.baselines,
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
    )?;
    writer.write_bits(0, 16)?;
    Ok(())
}

/// Native protocol-28 packetentities. An unavailable retained request receives
/// a native full update; this does not add an extension to the wire.
pub fn write_qw(
    writer: &mut Writer<'_>,
    ring: &QwRing,
    sequence: u32,
    delta_request: Option<(u32, u8)>,
) -> Result<(), packet::Error> {
    let to = ring.frame(sequence).ok_or(packet::Error::Context)?;
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
    let to = ring.frame(sequence).ok_or(packet::Error::Context)?;
    let seconds = to.time.seconds() as f32;
    writer.write_bits(4, 8)?;
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
    encode: impl Fn(
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
