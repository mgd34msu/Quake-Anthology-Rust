//! Connection-owned native projections. Storage is reserved at connect;
//! snapshot retention never owns another engine player/entity implementation.
use crate::{
    commands::packet,
    message::{Reader, Writer},
    states,
};

pub const SLOTS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entity<const N: usize> {
    pub number: u32,
    pub words: [u32; N],
}

#[derive(Clone, Copy)]
struct Slot<const P: usize> {
    sequence: Option<u32>,
    time: i32,
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
        time: 0,
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
    pub time: i32,
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
        if slot.sequence != Some(sequence)
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
            time: frame.time,
            command: frame.command,
            flags: frame.flags,
            area_bytes: frame.areas.len(),
            count: frame.entities.len(),
            first_entity,
            player: *frame.player,
        };
        let index = frame.sequence as usize & (SLOTS - 1);
        self.areas[index * self.area_capacity..index * self.area_capacity + frame.areas.len()]
            .copy_from_slice(frame.areas);
        self.entities[index * self.capacity..index * self.capacity + frame.entities.len()]
            .copy_from_slice(frame.entities);
        self.commit(slot);
        Ok(())
    }
    fn commit(&mut self, slot: Slot<P>) {
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
        self.slots[index] = slot;
        self.latest = Some(sequence);
        self.counts.accepted += 1;
    }
}

pub type Q3Ring = Ring<{ states::PLAYER_WORDS }, { states::ENTITY_WORDS }>;
pub type Q3Frame<'a> = Frame<'a, { states::PLAYER_WORDS }, { states::ENTITY_WORDS }>;

/// Original svc_snapshot body after its opcode. Missing deltas are consumed
/// without publishing, so following server commands remain in the same stream.
pub fn read_q3(
    reader: &mut Reader<'_>,
    ring: &mut Q3Ring,
    sequence: u32,
    command: u32,
) -> Result<bool, packet::Error> {
    let time = reader.read_bits(32)? as i32;
    let distance = reader.read_bits(8)?;
    let base_sequence = sequence.saturating_sub(distance);
    let full = distance == 0 || base_sequence == 0;
    let flags = reader.read_bits(8)? as u8;
    let area_bytes = reader.read_bits(8)? as usize;
    if area_bytes > ring.area_capacity || area_bytes > 32 {
        return Err(packet::Error::Count);
    }
    for byte in &mut ring.scratch_areas[..area_bytes] {
        *byte = reader.read_bits(8)? as u8;
    }
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
    let mut old_index = 0;
    let mut count = 0;
    let mut overflow = false;
    let mut previous = None;
    loop {
        let number = reader.read_bits(10)? as u16;
        if number == 1023 {
            break;
        }
        if previous.is_some_and(|n| number <= n) {
            return Err(packet::Error::Count);
        }
        previous = Some(number);
        while old_index < old.count {
            let entity = ring.entities[base_index * ring.capacity + old_index];
            if entity.number >= u32::from(number) {
                break;
            }
            append(&mut ring.scratch, entity, &mut count, &mut overflow);
            old_index += 1;
        }
        let old_entity =
            (old_index < old.count).then(|| ring.entities[base_index * ring.capacity + old_index]);
        let from = if let Some(e) = old_entity.filter(|e| e.number == u32::from(number)) {
            old_index += 1;
            e.words
        } else {
            *ring
                .baselines
                .get(number as usize)
                .ok_or(packet::Error::Count)?
        };
        if let Some(words) = states::read_q3_entity_body(reader, number, &from)?.words {
            append(
                &mut ring.scratch,
                Entity {
                    number: u32::from(number),
                    words,
                },
                &mut count,
                &mut overflow,
            );
        }
    }
    while old_index < old.count {
        let entity = ring.entities[base_index * ring.capacity + old_index];
        append(&mut ring.scratch, entity, &mut count, &mut overflow);
        old_index += 1;
    }
    let first_entity = ring.parsed_rows;
    ring.parsed_rows = ring.parsed_rows.saturating_add(count as u64);
    if !base_valid {
        ring.counts.missing_base += 1;
    }
    if overflow {
        ring.counts.overflow += 1;
    }
    if !base_valid || overflow {
        return Ok(false);
    }
    // Scratch and slot storage are separate owned typed arrays, not a raw arena.
    let index = sequence as usize & (SLOTS - 1);
    ring.areas[index * ring.area_capacity..index * ring.area_capacity + area_bytes]
        .copy_from_slice(&ring.scratch_areas[..area_bytes]);
    ring.entities[index * ring.capacity..index * ring.capacity + count]
        .copy_from_slice(&ring.scratch[..count]);
    ring.commit(Slot {
        sequence: Some(sequence),
        time,
        command,
        flags,
        area_bytes,
        count,
        first_entity,
        player,
    });
    Ok(true)
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
    let entities = &to.entities[..to.entities.partition_point(|e| e.number < 1023)];
    let from = delta_request
        .filter(|n| *n > 0 && sequence.checked_sub(*n).is_some_and(|d| d < 29))
        .and_then(|n| ring.frame(n));
    writer.write_bits(7, 8)?;
    writer.write_bits(to.time as u32, 32)?;
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
    let old = from.map_or(&[][..], |f| {
        &f.entities[..f.entities.partition_point(|e| e.number < 1023)]
    });
    let mut a = 0;
    let mut b = 0;
    while a < old.len() || b < entities.len() {
        let old_number = old.get(a).map_or(u32::MAX, |e| e.number);
        let new_number = entities.get(b).map_or(u32::MAX, |e| e.number);
        if old_number == new_number {
            states::write_q3_entity(
                writer,
                new_number,
                &old[a].words,
                Some(&entities[b].words),
                false,
            )?;
            a += 1;
            b += 1;
        } else if new_number < old_number {
            states::write_q3_entity(
                writer,
                new_number,
                &ring.baselines[new_number as usize],
                Some(&entities[b].words),
                true,
            )?;
            b += 1;
        } else {
            states::write_q3_entity(writer, old_number, &old[a].words, None, true)?;
            a += 1;
        }
    }
    writer.write_bits(1023, 10)?;
    Ok(())
}
