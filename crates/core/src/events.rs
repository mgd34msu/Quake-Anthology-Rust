use crate::primitives::{
    ClientId, EffectEvent, ModuleId, PrintEvent, PrintKind, SoundEvent, TextId, TextLease,
};
use std::fmt::{self, Write};

#[derive(Clone, Copy, Debug)]
pub enum FrameEvent {
    Sound(SoundEvent),
    Effect(EffectEvent),
    Print(PrintEvent),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventStorageError {
    RingCapacity,
    TextCapacity,
    ConsumerCapacity,
    StaleText,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputTarget {
    Presentation,
    Client(ClientId),
    Module(ModuleId),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputConsumerId {
    slot: usize,
    generation: u64,
}
/// A channel adapter maps an actual native receipt into this internal token.
/// It is never serialized or inferred from a transmit watermark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeReceipt(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputSubmission {
    Unsent,
    BestEffort,
    Reliable(NativeReceipt),
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OutputCounters {
    pub submissions: u64,
    pub acknowledgements: u64,
    pub acknowledged_records: u64,
    pub retired_on_resync: u64,
    pub skipped_during_resync: u64,
    pub overflow: u64,
    pub resyncs: u64,
}
#[derive(Clone, Copy, Debug)]
pub struct OutputRecord {
    pub sequence: u64,
    pub event: FrameEvent,
}
pub struct OutputBatch {
    consumer: OutputConsumerId,
    next: u64,
    end: u64,
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Delivery {
    #[default]
    Done,
    Pending,
    Awaiting(NativeReceipt),
}
struct OutputSlot {
    record: OutputRecord,
    remaining: usize,
    text: Option<TextLease>,
}
#[derive(Default)]
struct OutputCursor {
    target: Option<OutputTarget>,
    generation: u64,
    cursor: u64,
    resync: bool,
    counters: OutputCounters,
}

/// One slot-owned payload store, with independent delivery state per consumer.
/// A full ring retires only lagging consumers into bounded resynchronization.
pub struct EventRing {
    slots: Box<[Option<OutputSlot>]>,
    consumers: Box<[OutputCursor]>,
    active: Box<[usize]>,
    active_len: usize,
    deliveries: Box<[Delivery]>,
    head: u64,
    tail: u64,
    mask: usize,
    rejected_publications: u64,
    pub texts: TextStore,
}
impl EventRing {
    pub fn load(
        capacity: usize,
        consumers: usize,
        text_rows: usize,
        text_width: usize,
    ) -> Result<Self, EventStorageError> {
        if !capacity.is_power_of_two() || capacity > 65536 {
            return Err(EventStorageError::RingCapacity);
        }
        let size = capacity
            .checked_mul(consumers)
            .filter(|&n| consumers != 0 && n <= 16 * 1024 * 1024)
            .ok_or(EventStorageError::ConsumerCapacity)?;
        Ok(Self {
            slots: std::iter::repeat_with(|| None).take(capacity).collect(),
            consumers: std::iter::repeat_with(OutputCursor::default)
                .take(consumers)
                .collect(),
            deliveries: vec![Delivery::Done; size].into_boxed_slice(),
            active: vec![0; consumers].into_boxed_slice(),
            active_len: 0,
            head: 0,
            tail: 0,
            mask: capacity - 1,
            rejected_publications: 0,
            texts: TextStore::load(text_rows, text_width)?,
        })
    }
    pub fn bind(&mut self, target: OutputTarget) -> Option<OutputConsumerId> {
        if self.consumers.iter().any(|c| c.target == Some(target)) {
            return None;
        }
        let slot = self
            .consumers
            .iter()
            .position(|c| c.target.is_none() && c.generation != u64::MAX)?;
        let c = &mut self.consumers[slot];
        c.generation += 1;
        c.target = Some(target);
        c.cursor = self.tail;
        c.resync = false;
        c.counters = OutputCounters::default();
        self.active[self.active_len] = slot;
        self.active_len += 1;
        Some(OutputConsumerId {
            slot,
            generation: c.generation,
        })
    }
    fn valid(&self, id: OutputConsumerId) -> bool {
        self.consumers
            .get(id.slot)
            .is_some_and(|c| c.target.is_some() && c.generation == id.generation)
    }
    pub fn counters(&self, id: OutputConsumerId) -> Option<OutputCounters> {
        self.valid(id).then(|| self.consumers[id.slot].counters)
    }
    pub fn needs_resync(&self, id: OutputConsumerId) -> bool {
        self.valid(id) && self.consumers[id.slot].resync
    }
    /// Call only after native resync has completed (or for an in-process sink).
    /// Changing the binding generation invalidates all receipts from its old epoch.
    pub fn resume(&mut self, id: OutputConsumerId) -> Option<OutputConsumerId> {
        if !self.needs_resync(id) || id.generation == u64::MAX {
            return None;
        }
        let c = &mut self.consumers[id.slot];
        c.generation += 1;
        c.resync = false;
        c.cursor = self.tail;
        Some(OutputConsumerId {
            slot: id.slot,
            generation: c.generation,
        })
    }
    pub fn unbind(&mut self, id: OutputConsumerId) {
        if self.valid(id) {
            self.cancel_pending(id.slot);
            self.consumers[id.slot].target = None;
            if let Some(index) = self.active[..self.active_len]
                .iter()
                .position(|&slot| slot == id.slot)
            {
                self.active.copy_within(index + 1..self.active_len, index);
                self.active_len -= 1;
            }
            self.retire();
        }
    }
    fn offset(&self, consumer: usize, sequence: u64) -> usize {
        consumer * self.slots.len() + (sequence as usize & self.mask)
    }
    fn cancel_pending(&mut self, consumer: usize) -> usize {
        let mut cancelled = 0;
        for sequence in self.head..self.tail {
            let offset = self.offset(consumer, sequence);
            if self.deliveries[offset] != Delivery::Done {
                self.deliveries[offset] = Delivery::Done;
                cancelled += 1;
                if let Some(slot) = &mut self.slots[sequence as usize & self.mask] {
                    slot.remaining -= 1;
                }
            }
        }
        self.consumers[consumer].cursor = self.tail;
        cancelled
    }
    fn retire(&mut self) {
        while self.head < self.tail {
            let index = self.head as usize & self.mask;
            if self.slots[index].as_ref().is_some_and(|s| s.remaining != 0) {
                break;
            }
            if let Some(slot) = self.slots[index].take()
                && let Some(text) = slot.text
            {
                self.texts.release(text);
            }
            self.head += 1;
        }
    }
    fn make_room(&mut self) {
        self.retire();
        if self.len() != self.slots.len() {
            return;
        }
        for index in 0..self.active_len {
            let consumer = self.active[index];
            if self.deliveries[self.offset(consumer, self.head)] != Delivery::Done {
                let c = &mut self.consumers[consumer];
                c.counters.overflow += 1;
                c.counters.resyncs += 1;
                c.resync = true;
                let retired = self.cancel_pending(consumer);
                self.consumers[consumer].counters.retired_on_resync += retired as u64;
            }
        }
        self.retire();
    }
    pub fn push(&mut self, event: FrameEvent) -> Result<u64, EventStorageError> {
        self.make_room();
        let text = match event {
            FrameEvent::Print(p) => {
                let Some(lease) = self.texts.lease(p.text) else {
                    self.rejected_publications += 1;
                    return Err(EventStorageError::StaleText);
                };
                Some(lease)
            }
            _ => None,
        };
        let sequence = self.tail;
        let mut remaining = 0;
        for index in 0..self.active_len {
            let consumer = self.active[index];
            let c = &mut self.consumers[consumer];
            let relevant = c.target.is_some()
                && match (c.target, event) {
                    (Some(OutputTarget::Client(id)), FrameEvent::Print(p)) => {
                        p.client.is_none_or(|to| to == id)
                    }
                    _ => true,
                };
            if relevant && c.resync {
                c.counters.skipped_during_resync += 1;
            }
            let accepts = relevant && !c.resync;
            let offset = self.offset(consumer, sequence);
            self.deliveries[offset] = if accepts {
                remaining += 1;
                Delivery::Pending
            } else {
                Delivery::Done
            };
        }
        self.slots[sequence as usize & self.mask] = Some(OutputSlot {
            record: OutputRecord { sequence, event },
            remaining,
            text,
        });
        self.tail += 1;
        self.retire();
        Ok(sequence)
    }
    pub fn print(
        &mut self,
        client: Option<ClientId>,
        kind: PrintKind,
        text: fmt::Arguments<'_>,
    ) -> Result<u64, EventStorageError> {
        self.publish_print(client, kind, |texts| texts.insert_formatted(text))
    }
    /// Native module strings are byte strings, including palette-font glyphs.
    pub fn print_bytes(
        &mut self,
        client: Option<ClientId>,
        kind: PrintKind,
        text: &[u8],
    ) -> Result<u64, EventStorageError> {
        self.publish_print(client, kind, |texts| texts.insert(text))
    }
    fn publish_print(
        &mut self,
        client: Option<ClientId>,
        kind: PrintKind,
        write: impl FnOnce(&mut TextStore) -> Option<TextLease>,
    ) -> Result<u64, EventStorageError> {
        self.make_room();
        let Some(lease) = write(&mut self.texts) else {
            self.rejected_publications += 1;
            return Err(EventStorageError::TextCapacity);
        };
        let result = self.push(FrameEvent::Print(PrintEvent {
            client,
            kind,
            text: lease.id(),
        }));
        self.texts.release(lease);
        result
    }
    pub fn batch(&self, consumer: OutputConsumerId) -> Option<OutputBatch> {
        (self.valid(consumer) && !self.consumers[consumer.slot].resync).then(|| OutputBatch {
            consumer,
            next: self.consumers[consumer.slot].cursor.max(self.head),
            end: self.tail,
        })
    }
    /// An unsent record is skipped only for this bounded pass, and retried later.
    pub fn next(&self, batch: &mut OutputBatch) -> Option<OutputRecord> {
        if !self.valid(batch.consumer) || self.consumers[batch.consumer.slot].resync {
            return None;
        }
        batch.next = batch.next.max(self.head);
        while batch.next < batch.end.min(self.tail) {
            let sequence = batch.next;
            batch.next += 1;
            if self.deliveries[self.offset(batch.consumer.slot, sequence)] == Delivery::Pending {
                return self.slots[sequence as usize & self.mask]
                    .as_ref()
                    .map(|s| s.record);
            }
        }
        None
    }
    fn advance(&mut self, id: OutputConsumerId) {
        let mut cursor = self.consumers[id.slot].cursor.max(self.head);
        while cursor < self.tail && self.deliveries[self.offset(id.slot, cursor)] == Delivery::Done
        {
            cursor += 1;
        }
        self.consumers[id.slot].cursor = cursor;
        self.retire();
    }
    pub fn submit(
        &mut self,
        consumer: OutputConsumerId,
        sequence: u64,
        submission: OutputSubmission,
    ) -> bool {
        if !self.valid(consumer)
            || self.consumers[consumer.slot].resync
            || sequence < self.head
            || sequence >= self.tail
        {
            return false;
        }
        let offset = self.offset(consumer.slot, sequence);
        if self.deliveries[offset] != Delivery::Pending {
            return false;
        }
        match submission {
            OutputSubmission::Unsent => return true,
            OutputSubmission::BestEffort => {
                self.deliveries[offset] = Delivery::Done;
                if let Some(slot) = &mut self.slots[sequence as usize & self.mask] {
                    slot.remaining -= 1;
                }
            }
            OutputSubmission::Reliable(receipt) => {
                self.deliveries[offset] = Delivery::Awaiting(receipt)
            }
        }
        self.consumers[consumer.slot].counters.submissions += 1;
        self.advance(consumer);
        true
    }
    /// Receipts are supplied by native ACK processing, never physical intake here.
    pub fn acknowledge(&mut self, consumer: OutputConsumerId, receipt: NativeReceipt) -> usize {
        if !self.valid(consumer) || self.consumers[consumer.slot].resync {
            return 0;
        }
        let mut count = 0;
        for sequence in self.consumers[consumer.slot].cursor.max(self.head)..self.tail {
            let offset = self.offset(consumer.slot, sequence);
            if self.deliveries[offset] == Delivery::Awaiting(receipt) {
                self.deliveries[offset] = Delivery::Done;
                if let Some(slot) = &mut self.slots[sequence as usize & self.mask] {
                    slot.remaining -= 1;
                }
                count += 1;
            }
        }
        self.consumers[consumer.slot].counters.acknowledgements += u64::from(count != 0);
        self.consumers[consumer.slot].counters.acknowledged_records += count as u64;
        self.advance(consumer);
        count
    }
    pub fn rejected_publications(&self) -> u64 {
        self.rejected_publications
    }
    pub fn len(&self) -> usize {
        (self.tail - self.head) as usize
    }
    pub fn is_empty(&self) -> bool {
        self.head == self.tail
    }
}

/// A page is reusable only after its event and every display lease release it.
pub struct TextStore {
    bytes: Box<[u8]>,
    lengths: Box<[u16]>,
    generations: Box<[u32]>,
    refs: Box<[u32]>,
    width: usize,
    next: usize,
    reused: u64,
    truncated: u64,
}
impl TextStore {
    pub fn load(rows: usize, width: usize) -> Result<Self, EventStorageError> {
        if rows == 0
            || rows > u16::MAX as usize
            || width == 0
            || width > u16::MAX as usize
            || rows
                .checked_mul(width)
                .is_none_or(|size| size > 64 * 1024 * 1024)
        {
            return Err(EventStorageError::TextCapacity);
        }
        Ok(Self {
            bytes: vec![0; rows * width].into_boxed_slice(),
            lengths: vec![0; rows].into_boxed_slice(),
            generations: vec![0; rows].into_boxed_slice(),
            refs: vec![0; rows].into_boxed_slice(),
            width,
            next: 0,
            reused: 0,
            truncated: 0,
        })
    }
    fn reserve(&mut self) -> Option<(usize, TextLease)> {
        let rows = self.lengths.len();
        let slot = (0..rows)
            .map(|offset| (self.next + offset) % rows)
            .find(|&slot| self.refs[slot] == 0 && self.generations[slot] != u32::MAX)?;
        self.next = (slot + 1) % rows;
        self.reused += u64::from(self.generations[slot] != 0);
        self.generations[slot] += 1;
        self.refs[slot] = 1;
        Some((
            slot,
            TextLease {
                id: TextId {
                    slot: slot as u16,
                    generation: self.generations[slot],
                },
            },
        ))
    }
    pub fn insert(&mut self, text: &[u8]) -> Option<TextLease> {
        let (slot, lease) = self.reserve()?;
        let len = text.len().min(self.width);
        self.truncated += u64::from(len != text.len());
        self.bytes[slot * self.width..slot * self.width + len].copy_from_slice(&text[..len]);
        self.lengths[slot] = len as u16;
        Some(lease)
    }
    pub fn insert_formatted(&mut self, text: fmt::Arguments<'_>) -> Option<TextLease> {
        let (slot, lease) = self.reserve()?;
        let mut writer = TextWriter {
            bytes: &mut self.bytes[slot * self.width..(slot + 1) * self.width],
            len: 0,
            truncated: false,
        };
        let _ = writer.write_fmt(text);
        self.lengths[slot] = writer.len as u16;
        self.truncated += u64::from(writer.truncated);
        Some(lease)
    }
    pub fn lease(&mut self, id: TextId) -> Option<TextLease> {
        self.get(id)?;
        self.refs[id.slot as usize] = self.refs[id.slot as usize].checked_add(1)?;
        Some(TextLease { id })
    }
    pub fn release(&mut self, lease: TextLease) {
        if self.get(lease.id).is_some() {
            self.refs[lease.id.slot as usize] -= 1;
        }
    }
    pub fn get(&self, id: TextId) -> Option<&[u8]> {
        let slot = id.slot as usize;
        if id.generation == 0
            || self.generations.get(slot).copied()? != id.generation
            || self.refs[slot] == 0
        {
            return None;
        }
        Some(&self.bytes[slot * self.width..slot * self.width + self.lengths[slot] as usize])
    }
    pub fn reused(&self) -> u64 {
        self.reused
    }
    pub fn truncated(&self) -> u64 {
        self.truncated
    }
}

struct TextWriter<'a> {
    bytes: &'a mut [u8],
    len: usize,
    truncated: bool,
}
impl Write for TextWriter<'_> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        if self.truncated {
            return Ok(());
        }
        let mut count = text.len().min(self.bytes.len() - self.len);
        while !text.is_char_boundary(count) {
            count -= 1;
        }
        self.bytes[self.len..self.len + count].copy_from_slice(&text.as_bytes()[..count]);
        self.len += count;
        self.truncated = count != text.len();
        Ok(())
    }
}
