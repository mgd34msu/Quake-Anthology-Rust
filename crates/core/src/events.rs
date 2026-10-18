use crate::primitives::{
    ClientId, EffectEvent, EntityId, EntityRecord, FileChunk, HudEvent, IndexSpan, ModuleId,
    NATIVE_CUE_CAPTURE_BYTES, NATIVE_PAYLOAD_BYTES, NATIVE_PAYLOAD_PAGES, NATIVE_PEER_ROUTE_BYTES,
    NATIVE_WIRE_BYTES, NATIVE_WIRE_PAGES, NativeDelivery, NativePayload, NativeTarget,
    NumericValue, PrintEvent, PrintKind, PrintText, RuleSetId, SoundEvent, TextId, TextLease,
    ValueId, Vec3, ViewFeedbackEvent,
};
use std::fmt::{self, Write};

/// Independent display leases reserved for the one console's retained output.
pub const CONSOLE_DISPLAY_RECORDS: usize = 128;

#[derive(Clone, Copy, Debug)]
#[expect(
    clippy::large_enum_variant,
    reason = "Fixed ring slots retain native page handles without allocating on publication."
)]
pub enum FrameEvent {
    ResetPresentation {
        client: ClientId,
        lifetime: EntityId,
    },
    Sound(SoundEvent),
    Music(crate::primitives::MusicControl),
    StaticEntity(EntityRecord),
    Effect(EffectEvent),
    Print(PrintEvent),
    Hud(HudEvent),
    ViewFeedback(ViewFeedbackEvent),
    PlayerValue {
        id: ValueId,
        value: NumericValue,
    },
    SetAngles(Vec3),
    Native(NativePayload),
    /// Reliable engine-produced SERVER peer metadata with captured receiver ordinals.
    PeerInfo {
        user_id: i32,
        payload: NativePayload,
    },
    /// Engine-produced file bytes. The native adapter selects framing and
    /// retirement policy; an unavailable file differs from a real empty block.
    FileChunk {
        receiver: EntityId,
        chunk: FileChunk<NativePayload>,
    },
    /// One native SERVER state body, retained until its channel acknowledges
    /// receipt. Framing is regenerated without changing its command watermark.
    GameState {
        payload: NativePayload,
        command_sequence: u32,
        body_bits: usize,
    },
    ConfigString {
        module: ModuleId,
        source: RuleSetId,
        index: u32,
        destination_index: u32,
        text: TextId,
    },
    /// One native SERVER command. Its ring-owned text remains leased until
    /// every addressed client channel reports a real reliable ACK.
    ServerCommand {
        module: ModuleId,
        source: RuleSetId,
        text: TextId,
    },
}
fn print_event(client: Option<ClientId>, kind: PrintKind, level: u8, text: TextId) -> FrameEvent {
    FrameEvent::Print(PrintEvent {
        client,
        kind,
        level,
        text: text.into(),
    })
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
    /// Local presentation and the selected CLIENT's existing HUD cursor.
    /// Received records never enter SERVER/module delivery through this scope.
    Local(ClientId),
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
    pub delivery: Option<NativeDelivery>,
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
    target: Option<OutputTarget>,
    remaining: usize,
    texts: [Option<TextLease>; NATIVE_PAYLOAD_PAGES],
    fresh_pages: u32,
}
/// A group stages into the ring's own unused slots, invisible until commit.
pub struct OutputGroup<'a> {
    ring: &'a mut EventRing,
    next: u64,
    end: u64,
}
impl OutputGroup<'_> {
    pub fn push_delivered(
        &mut self,
        delivery: NativeDelivery,
        event: FrameEvent,
    ) -> Result<u64, EventStorageError> {
        self.room()?;
        let texts = self.ring.lease_event(event)?;
        Ok(self.stage(delivery, event, texts, 0))
    }
    pub fn native_message_parts(
        &mut self,
        delivery: NativeDelivery,
        wire: &[u8],
        captures: &[u8],
    ) -> Result<u64, EventStorageError> {
        if !captures.len().is_multiple_of(NATIVE_CUE_CAPTURE_BYTES) {
            return Err(EventStorageError::TextCapacity);
        }
        self.payload(delivery, wire, captures, FrameEvent::Native)
    }
    pub fn print(
        &mut self,
        delivery: NativeDelivery,
        kind: PrintKind,
        level: u8,
        wire: &[u8],
        text: IndexSpan,
        consumed: u32,
    ) -> Result<u64, EventStorageError> {
        if consumed as usize > wire.len() || !text.fits(consumed as usize) {
            return Err(EventStorageError::TextCapacity);
        }
        self.payload(delivery, wire, &[], |wire| {
            FrameEvent::Print(PrintEvent {
                client: None,
                kind,
                level,
                text: PrintText::Native {
                    wire,
                    text,
                    consumed,
                },
            })
        })
    }
    fn payload(
        &mut self,
        delivery: NativeDelivery,
        wire: &[u8],
        captures: &[u8],
        event: impl FnOnce(NativePayload) -> FrameEvent,
    ) -> Result<u64, EventStorageError> {
        self.room()?;
        let (payload, texts) = self.ring.native_payload(wire, captures)?;
        let fresh = (1u32 << texts.iter().flatten().count()) - 1;
        Ok(self.stage(delivery, event(payload), texts, fresh))
    }
    fn room(&self) -> Result<(), EventStorageError> {
        if self.next == self.end {
            Err(EventStorageError::RingCapacity)
        } else {
            Ok(())
        }
    }
    fn stage(
        &mut self,
        delivery: NativeDelivery,
        event: FrameEvent,
        texts: [Option<TextLease>; NATIVE_PAYLOAD_PAGES],
        fresh_pages: u32,
    ) -> u64 {
        let sequence = self.next;
        self.ring
            .store_slot(sequence, None, Some(delivery), event, texts, fresh_pages);
        self.next += 1;
        sequence
    }
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
    /// The caller must have completed native resync, as for `resume`.
    pub fn resume_if_needed(&mut self, id: &mut OutputConsumerId) -> bool {
        if !self.valid(*id) {
            return false;
        }
        if self.needs_resync(*id) {
            let Some(resumed) = self.resume(*id) else {
                return false;
            };
            *id = resumed;
        }
        true
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
            if let Some(slot) = self.slots[index].take() {
                self.texts.release_all(slot.texts);
            }
            self.head += 1;
        }
    }
    fn make_room(&mut self, count: usize) {
        self.retire();
        while self.slots.len() - self.len() < count {
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
    }
    pub fn push(&mut self, event: FrameEvent) -> Result<u64, EventStorageError> {
        self.publish(None, None, event)
    }
    pub fn push_to(
        &mut self,
        target: OutputTarget,
        event: FrameEvent,
    ) -> Result<u64, EventStorageError> {
        self.publish(Some(target), None, event)
    }
    /// SERVER emissions reach native CLIENT channels before presentation.
    pub fn push_delivered(
        &mut self,
        delivery: NativeDelivery,
        event: FrameEvent,
    ) -> Result<u64, EventStorageError> {
        self.publish(None, Some(delivery), event)
    }
    fn publish(
        &mut self,
        target: Option<OutputTarget>,
        delivery: Option<NativeDelivery>,
        event: FrameEvent,
    ) -> Result<u64, EventStorageError> {
        self.make_room(1);
        let texts = self.lease_event(event).inspect_err(|_| {
            self.rejected_publications += 1;
        })?;
        let sequence = self.tail;
        self.store_slot(sequence, target, delivery, event, texts, 0);
        self.commit_slot(sequence);
        self.tail += 1;
        self.retire();
        Ok(sequence)
    }
    fn lease_event(
        &mut self,
        event: FrameEvent,
    ) -> Result<[Option<TextLease>; NATIVE_PAYLOAD_PAGES], EventStorageError> {
        let mut texts = [const { None }; NATIVE_PAYLOAD_PAGES];
        let mut pages = [None; NATIVE_PAYLOAD_PAGES];
        match event {
            FrameEvent::Print(p) => match p.text {
                PrintText::Text(id) => pages[0] = Some(id),
                PrintText::Native { wire, .. } => pages = wire.pages,
            },
            FrameEvent::ConfigString { text, .. } | FrameEvent::ServerCommand { text, .. } => {
                pages[0] = Some(text);
            }
            FrameEvent::Music(crate::primitives::MusicControl::Start {
                intro, loop_track, ..
            }) => {
                pages[0] = Some(intro);
                pages[1] = Some(loop_track);
            }
            FrameEvent::Music(crate::primitives::MusicControl::Remap { entries }) => {
                pages[0] = Some(entries);
            }
            FrameEvent::Native(payload)
            | FrameEvent::PeerInfo { payload, .. }
            | FrameEvent::GameState { payload, .. } => {
                pages = payload.pages;
            }
            FrameEvent::FileChunk { chunk, .. } => {
                if let Some(payload) = chunk.payload() {
                    pages = payload.pages;
                }
            }
            _ => {}
        }
        for (index, page) in pages.into_iter().enumerate() {
            if let Some(page) = page {
                let Some(lease) = self.texts.lease(page) else {
                    self.texts.release_all(texts);
                    return Err(EventStorageError::StaleText);
                };
                texts[index] = Some(lease);
            }
        }
        Ok(texts)
    }
    fn store_slot(
        &mut self,
        sequence: u64,
        target: Option<OutputTarget>,
        delivery: Option<NativeDelivery>,
        event: FrameEvent,
        texts: [Option<TextLease>; NATIVE_PAYLOAD_PAGES],
        fresh_pages: u32,
    ) {
        self.slots[sequence as usize & self.mask] = Some(OutputSlot {
            record: OutputRecord {
                sequence,
                event,
                delivery,
            },
            target,
            remaining: 0,
            texts,
            fresh_pages,
        });
    }
    fn commit_slot(&mut self, sequence: u64) {
        let Some(slot) = self.slots[sequence as usize & self.mask].as_ref() else {
            return;
        };
        let target = slot.target;
        let delivery = slot.record.delivery;
        let event = slot.record.event;
        let mut remaining = 0;
        for index in 0..self.active_len {
            let consumer = self.active[index];
            let c = &mut self.consumers[consumer];
            let relevant = c.target.is_some()
                && match c.target {
                    Some(OutputTarget::Local(id)) => target == Some(OutputTarget::Local(id)),
                    _ => true,
                }
                && target.is_none_or(|target| match target {
                    OutputTarget::Local(id) => {
                        c.target == Some(OutputTarget::Presentation)
                            || c.target == Some(OutputTarget::Client(id))
                            || c.target == Some(target)
                    }
                    _ => c.target == Some(target),
                })
                && match event {
                    FrameEvent::PeerInfo { .. } | FrameEvent::FileChunk { .. } => {
                        matches!(c.target, Some(OutputTarget::Client(_)))
                    }
                    _ => true,
                }
                && delivery.is_none_or(|delivery| match c.target {
                    Some(OutputTarget::Client(id)) => match delivery.target {
                        NativeTarget::Client(to) => id == to,
                        _ => true,
                    },
                    Some(OutputTarget::Module(_)) => true,
                    _ => false,
                })
                && (delivery.is_some()
                    || match (c.target, event) {
                        (
                            Some(OutputTarget::Client(id) | OutputTarget::Local(id)),
                            FrameEvent::Sound(sound),
                        ) => sound.applies_to_client(id),
                        (
                            Some(OutputTarget::Client(id) | OutputTarget::Local(id)),
                            FrameEvent::Effect(effect),
                        ) => effect.applies_to_client(id),
                        (
                            Some(OutputTarget::Client(id) | OutputTarget::Local(id)),
                            FrameEvent::Print(p),
                        ) => p.client.is_none_or(|to| to == id),
                        (
                            Some(OutputTarget::Client(id) | OutputTarget::Local(id)),
                            FrameEvent::Hud(event),
                        ) => event.client == id,
                        (
                            Some(OutputTarget::Client(id) | OutputTarget::Local(id)),
                            FrameEvent::ViewFeedback(event),
                        ) => event.client == id,
                        (Some(OutputTarget::Client(_)), FrameEvent::ServerCommand { .. }) => true,
                        (_, FrameEvent::ServerCommand { .. }) => false,
                        _ => true,
                    });
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
        if let Some(slot) = self.slots[sequence as usize & self.mask].as_mut() {
            slot.remaining = remaining;
            slot.fresh_pages = 0;
        }
    }
    /// Reserve room using the same bounded slow-consumer resync as a single
    /// publication. Staging failure leaves no visible prefix or text lease.
    pub fn publish_group(
        &mut self,
        count: usize,
        fill: impl FnOnce(&mut OutputGroup<'_>) -> Result<(), EventStorageError>,
    ) -> Result<std::ops::Range<u64>, EventStorageError> {
        if count > self.slots.len() || count == 0 {
            self.rejected_publications += 1;
            return Err(EventStorageError::RingCapacity);
        }
        self.make_room(count);
        let start = self.tail;
        let end = start + count as u64;
        let checkpoint = self.texts.checkpoint();
        let mut group = OutputGroup {
            ring: self,
            next: start,
            end,
        };
        let result = fill(&mut group).and_then(|()| {
            (group.next == end)
                .then_some(())
                .ok_or(EventStorageError::RingCapacity)
        });
        let staged_end = group.next;
        if let Err(error) = result {
            for sequence in start..staged_end {
                if let Some(slot) = self.slots[sequence as usize & self.mask].take() {
                    for (index, lease) in slot.texts.into_iter().enumerate() {
                        if let Some(lease) = lease {
                            if slot.fresh_pages & (1 << index) != 0 {
                                self.texts.rollback_new(lease);
                            } else {
                                self.texts.release(lease);
                            }
                        }
                    }
                }
            }
            self.texts.restore_checkpoint(checkpoint);
            self.rejected_publications += 1;
            return Err(error);
        }
        for sequence in start..end {
            self.commit_slot(sequence);
        }
        self.tail = end;
        self.retire();
        Ok(start..end)
    }
    /// A bounded producer staging failure is a rejected ring publication.
    pub fn reject_publication(&mut self) {
        self.rejected_publications += 1;
    }
    pub fn native_message(
        &mut self,
        delivery: NativeDelivery,
        bytes: &[u8],
    ) -> Result<u64, EventStorageError> {
        self.native_message_bound(delivery, bytes.len(), bytes)
    }
    /// Captured cue bindings are appended by the native boundary to these
    /// same owned pages. The wire length excludes their immutable metadata.
    pub fn native_message_bound(
        &mut self,
        delivery: NativeDelivery,
        wire_length: usize,
        bytes: &[u8],
    ) -> Result<u64, EventStorageError> {
        let Some((wire, captures)) = bytes.split_at_checked(wire_length) else {
            self.rejected_publications += 1;
            return Err(EventStorageError::TextCapacity);
        };
        if !captures.len().is_multiple_of(NATIVE_CUE_CAPTURE_BYTES) {
            self.rejected_publications += 1;
            return Err(EventStorageError::TextCapacity);
        }
        self.publish_payload(None, Some(delivery), wire, captures, FrameEvent::Native)
    }
    pub fn game_state(
        &mut self,
        delivery: NativeDelivery,
        command_sequence: u32,
        body_bits: usize,
        bytes: &[u8],
    ) -> Result<u64, EventStorageError> {
        if body_bits == 0 || body_bits > bytes.len().saturating_mul(8) {
            self.rejected_publications += 1;
            return Err(EventStorageError::TextCapacity);
        }
        self.publish_payload(None, Some(delivery), bytes, &[], |payload| {
            FrameEvent::GameState {
                payload,
                command_sequence,
                body_bits,
            }
        })
    }
    pub fn peer_info(
        &mut self,
        user_id: i32,
        info: &[u8],
        captures: &[u8],
    ) -> Result<u64, EventStorageError> {
        if !captures.len().is_multiple_of(NATIVE_PEER_ROUTE_BYTES) {
            self.rejected_publications += 1;
            return Err(EventStorageError::TextCapacity);
        }
        self.publish_payload(None, None, info, captures, |payload| FrameEvent::PeerInfo {
            user_id,
            payload,
        })
    }
    pub fn file_chunk(
        &mut self,
        client: ClientId,
        receiver: EntityId,
        chunk: FileChunk<&[u8]>,
    ) -> Result<u64, EventStorageError> {
        if matches!(chunk, FileChunk::Sequential { percent: 101.., .. }) {
            self.rejected_publications += 1;
            return Err(EventStorageError::TextCapacity);
        }
        self.publish_payload(
            Some(OutputTarget::Client(client)),
            None,
            chunk.payload().copied().unwrap_or(&[]),
            &[],
            |payload| FrameEvent::FileChunk {
                receiver,
                chunk: chunk.map(|_| payload),
            },
        )
    }
    fn publish_payload(
        &mut self,
        target: Option<OutputTarget>,
        delivery: Option<NativeDelivery>,
        wire: &[u8],
        captures: &[u8],
        event: impl FnOnce(NativePayload) -> FrameEvent,
    ) -> Result<u64, EventStorageError> {
        self.make_room(1);
        let (payload, texts) = self.native_payload(wire, captures).inspect_err(|_| {
            self.rejected_publications += 1;
        })?;
        let sequence = self.tail;
        self.store_slot(sequence, target, delivery, event(payload), texts, 0);
        self.commit_slot(sequence);
        self.tail += 1;
        self.retire();
        Ok(sequence)
    }
    fn native_payload(
        &mut self,
        wire: &[u8],
        captures: &[u8],
    ) -> Result<(NativePayload, [Option<TextLease>; NATIVE_PAYLOAD_PAGES]), EventStorageError> {
        let stored_length = wire
            .len()
            .checked_add(captures.len())
            .ok_or(EventStorageError::TextCapacity)?;
        if wire.len() > NATIVE_WIRE_BYTES.min(self.texts.width * NATIVE_WIRE_PAGES)
            || stored_length > NATIVE_PAYLOAD_BYTES.min(self.texts.width * NATIVE_PAYLOAD_PAGES)
        {
            return Err(EventStorageError::TextCapacity);
        }
        let checkpoint = self.texts.checkpoint();
        let mut leases = [const { None }; NATIVE_PAYLOAD_PAGES];
        let mut pages = [None; NATIVE_PAYLOAD_PAGES];
        for index in 0..stored_length.div_ceil(self.texts.width) {
            let start = index * self.texts.width;
            let end = (start + self.texts.width).min(stored_length);
            let first = wire.get(start..end.min(wire.len())).unwrap_or(&[]);
            let second =
                &captures[start.saturating_sub(wire.len())..end.saturating_sub(wire.len())];
            let Some(lease) = self.texts.insert_parts(&[first, second]) else {
                for lease in leases.into_iter().flatten() {
                    self.texts.rollback_new(lease);
                }
                self.texts.restore_checkpoint(checkpoint);
                return Err(EventStorageError::TextCapacity);
            };
            pages[index] = Some(lease.id());
            leases[index] = Some(lease);
        }
        Ok((
            NativePayload {
                pages,
                length: wire.len(),
                stored_length,
            },
            leases,
        ))
    }
    pub fn print(
        &mut self,
        client: Option<ClientId>,
        kind: PrintKind,
        text: fmt::Arguments<'_>,
    ) -> Result<u64, EventStorageError> {
        self.publish_print(None, client, kind, kind.default_level(), |texts| {
            texts.insert_formatted(text)
        })
    }
    /// Configuration is exact text, unlike a display print which may truncate.
    pub fn publish_configstring(
        &mut self,
        target: Option<OutputTarget>,
        module: ModuleId,
        source: RuleSetId,
        index: u32,
        destination_index: u32,
        text: &[u8],
    ) -> Result<u64, EventStorageError> {
        if text.len() > self.texts.width || text.contains(&0) {
            self.rejected_publications += 1;
            return Err(EventStorageError::TextCapacity);
        }
        self.publish_text(
            target,
            |texts| texts.insert(text),
            |text| FrameEvent::ConfigString {
                module,
                source,
                index,
                destination_index,
                text,
            },
        )
    }
    /// Native command text is exact. The ABI adapter applies its source
    /// storage width before this shared ring takes the one retained copy.
    pub fn server_command(
        &mut self,
        client: Option<ClientId>,
        module: ModuleId,
        source: RuleSetId,
        text: &[u8],
    ) -> Result<u64, EventStorageError> {
        if text.len() > self.texts.width || text.contains(&0) {
            self.rejected_publications += 1;
            return Err(EventStorageError::TextCapacity);
        }
        self.publish_text(
            client.map(OutputTarget::Client),
            |texts| texts.insert(text),
            |text| FrameEvent::ServerCommand {
                module,
                source,
                text,
            },
        )
    }
    /// All music controls use the presentation consumer's existing cursor.
    pub fn music_control(
        &mut self,
        control: crate::primitives::MusicControl,
    ) -> Result<u64, EventStorageError> {
        self.push_to(OutputTarget::Presentation, FrameEvent::Music(control))
    }
    /// A complete remap update is one atomic presentation publication.
    pub fn music_remap(&mut self, entries: &[u8]) -> Result<u64, EventStorageError> {
        if crate::primitives::MusicRemap::parse(entries).is_none() {
            self.rejected_publications += 1;
            return Err(EventStorageError::TextCapacity);
        }
        self.publish_text(
            Some(OutputTarget::Presentation),
            |texts| texts.insert(entries),
            |entries| FrameEvent::Music(crate::primitives::MusicControl::Remap { entries }),
        )
    }
    /// Both track names belong to the same atomic output publication.
    pub fn music(
        &mut self,
        owner: crate::primitives::MusicOwner,
        playback: crate::primitives::MusicPlayback,
        intro: &[u8],
        loop_track: &[u8],
        file_search: Option<crate::primitives::FileSearchId>,
    ) -> Result<u64, EventStorageError> {
        if [intro, loop_track]
            .iter()
            .any(|text| text.len() > self.texts.width || text.contains(&0))
        {
            self.rejected_publications += 1;
            return Err(EventStorageError::TextCapacity);
        }
        self.make_room(1);
        let checkpoint = self.texts.checkpoint();
        let Some(first) = self.texts.insert(intro) else {
            self.rejected_publications += 1;
            return Err(EventStorageError::TextCapacity);
        };
        let Some(second) = self.texts.insert(loop_track) else {
            self.texts.rollback_new(first);
            self.texts.restore_checkpoint(checkpoint);
            self.rejected_publications += 1;
            return Err(EventStorageError::TextCapacity);
        };
        let result = self.music_control(crate::primitives::MusicControl::Start {
            file_search,
            owner,
            playback,
            intro: first.id(),
            loop_track: second.id(),
        });
        self.texts.release(first);
        self.texts.release(second);
        result
    }
    /// Native module strings are byte strings, including palette-font glyphs.
    pub fn print_bytes(
        &mut self,
        client: Option<ClientId>,
        kind: PrintKind,
        text: &[u8],
    ) -> Result<u64, EventStorageError> {
        self.print_level_bytes(None, client, kind, kind.default_level(), text)
    }
    pub fn print_level_bytes(
        &mut self,
        target: Option<OutputTarget>,
        client: Option<ClientId>,
        kind: PrintKind,
        level: u8,
        text: &[u8],
    ) -> Result<u64, EventStorageError> {
        self.publish_print(target, client, kind, level, |texts| texts.insert(text))
    }
    pub fn console_bytes(&mut self, text: &[u8]) -> Result<u64, EventStorageError> {
        self.console_parts(&[text])
    }
    pub fn console_parts(&mut self, parts: &[&[u8]]) -> Result<u64, EventStorageError> {
        self.publish_console(|texts| texts.insert_parts(parts))
    }
    pub fn console_formatted(
        &mut self,
        text: fmt::Arguments<'_>,
    ) -> Result<u64, EventStorageError> {
        self.publish_console(|texts| texts.insert_formatted(text))
    }
    /// Replaces this consumer's pending query in place, preserving print order.
    /// The response keeps its existing text lease and sequence.
    pub fn console_response(
        &mut self,
        consumer: OutputConsumerId,
        sequence: u64,
        lease: TextLease,
    ) -> bool {
        let valid = self.retained_for(consumer, sequence)
            && self.deliveries[self.offset(consumer.slot, sequence)] == Delivery::Pending
            && self.texts.get(lease.id()).is_some();
        if valid
            && let Some(slot) = &mut self.slots[sequence as usize & self.mask]
            && slot.record.sequence == sequence
            && slot.target == Some(OutputTarget::Presentation)
            && matches!(
                slot.record.event,
                FrameEvent::Music(crate::primitives::MusicControl::Report(_))
            )
        {
            slot.record.event = print_event(
                None,
                PrintKind::Console,
                PrintKind::Console.default_level(),
                lease.id(),
            );
            slot.texts[0] = Some(lease);
            return true;
        }
        self.texts.release(lease);
        self.rejected_publications += 1;
        false
    }
    fn publish_console(
        &mut self,
        write: impl FnOnce(&mut TextStore) -> Option<TextLease>,
    ) -> Result<u64, EventStorageError> {
        self.publish_print(
            Some(OutputTarget::Presentation),
            None,
            PrintKind::Console,
            PrintKind::Console.default_level(),
            write,
        )
    }
    fn publish_print(
        &mut self,
        target: Option<OutputTarget>,
        client: Option<ClientId>,
        kind: PrintKind,
        level: u8,
        write: impl FnOnce(&mut TextStore) -> Option<TextLease>,
    ) -> Result<u64, EventStorageError> {
        self.publish_text(target, write, |text| print_event(client, kind, level, text))
    }
    fn publish_text(
        &mut self,
        target: Option<OutputTarget>,
        write: impl FnOnce(&mut TextStore) -> Option<TextLease>,
        event: impl FnOnce(TextId) -> FrameEvent,
    ) -> Result<u64, EventStorageError> {
        self.make_room(1);
        let Some(lease) = write(&mut self.texts) else {
            self.rejected_publications += 1;
            return Err(EventStorageError::TextCapacity);
        };
        let result = self.publish(target, None, event(lease.id()));
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
        self.acknowledge_with(consumer, receipt, |_| {})
    }
    /// Observers see a proved native ACK before the owning slot is retired.
    pub fn acknowledge_with(
        &mut self,
        consumer: OutputConsumerId,
        receipt: NativeReceipt,
        mut observe: impl FnMut(OutputRecord),
    ) -> usize {
        if !self.valid(consumer) || self.consumers[consumer.slot].resync {
            return 0;
        }
        let mut count = 0;
        for sequence in self.consumers[consumer.slot].cursor.max(self.head)..self.tail {
            let offset = self.offset(consumer.slot, sequence);
            if self.deliveries[offset] == Delivery::Awaiting(receipt) {
                self.deliveries[offset] = Delivery::Done;
                if let Some(slot) = &mut self.slots[sequence as usize & self.mask] {
                    observe(slot.record);
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
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }
    pub fn sequence_range(&self) -> std::ops::Range<u64> {
        self.head..self.tail
    }
    /// True only while this consumer still owns the original record epoch.
    pub fn retained_for(&self, consumer: OutputConsumerId, sequence: u64) -> bool {
        self.valid(consumer)
            && !self.consumers[consumer.slot].resync
            && self.sequence_range().contains(&sequence)
            && self.deliveries[self.offset(consumer.slot, sequence)] != Delivery::Done
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
#[derive(Clone, Copy)]
struct TextCheckpoint {
    next: usize,
    reused: u64,
    truncated: u64,
}
impl TextStore {
    fn checkpoint(&self) -> TextCheckpoint {
        TextCheckpoint {
            next: self.next,
            reused: self.reused,
            truncated: self.truncated,
        }
    }
    fn restore_checkpoint(&mut self, checkpoint: TextCheckpoint) {
        self.next = checkpoint.next;
        self.reused = checkpoint.reused;
        self.truncated = checkpoint.truncated;
    }
    fn rollback_new(&mut self, lease: TextLease) {
        let slot = lease.id.slot as usize;
        self.release(lease);
        self.generations[slot] -= 1;
    }
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
        self.insert_parts(&[text])
    }
    fn insert_parts(&mut self, parts: &[&[u8]]) -> Option<TextLease> {
        let (slot, lease) = self.reserve()?;
        let wanted = parts
            .iter()
            .fold(0usize, |length, part| length.saturating_add(part.len()));
        let len = wanted.min(self.width);
        self.truncated += u64::from(len != wanted);
        let output = &mut self.bytes[slot * self.width..slot * self.width + len];
        let mut at = 0;
        for part in parts {
            let count = part.len().min(len - at);
            output[at..at + count].copy_from_slice(&part[..count]);
            at += count;
            if at == len {
                break;
            }
        }
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
    pub fn lease(&mut self, id: impl Into<PrintText>) -> Option<TextLease> {
        let id = id.into().display()?;
        self.get(id)?;
        self.refs[id.slot as usize] = self.refs[id.slot as usize].checked_add(1)?;
        Some(TextLease { id })
    }
    pub fn release(&mut self, lease: TextLease) {
        if self.get(lease.id).is_some() {
            self.refs[lease.id.slot as usize] -= 1;
        }
    }
    pub fn release_all<const N: usize>(&mut self, leases: [Option<TextLease>; N]) {
        for lease in leases.into_iter().flatten() {
            self.release(lease);
        }
    }
    pub fn get(&self, id: impl Into<PrintText>) -> Option<&[u8]> {
        let id = id.into().display()?;
        let slot = id.slot as usize;
        if id.generation == 0
            || self.generations.get(slot).copied()? != id.generation
            || self.refs[slot] == 0
        {
            return None;
        }
        Some(&self.bytes[slot * self.width..slot * self.width + self.lengths[slot] as usize])
    }
    pub fn copy_payload<'a>(
        &self,
        payload: NativePayload,
        output: &'a mut [u8],
    ) -> Option<&'a [u8]> {
        let output = output.get_mut(..payload.stored_length)?;
        let mut written = 0;
        for page in payload.pages.into_iter().flatten() {
            let bytes = self.get(page)?;
            output
                .get_mut(written..written + bytes.len())?
                .copy_from_slice(bytes);
            written += bytes.len();
        }
        (written == output.len()).then_some(output)
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

/// Consumers report actual successful submission, an actual native reliable
/// receipt, or no submission. Merely reading a record never retires it.
pub trait EventConsumer {
    fn submit(&mut self, record: OutputRecord, texts: &mut TextStore) -> OutputSubmission;
}
pub fn dispatch_consumer(
    events: &mut EventRing,
    id: OutputConsumerId,
    consumer: &mut impl EventConsumer,
) {
    dispatch_consumer_until(events, id, consumer, || false);
}

/// Stops before reading another record, preserving its pending delivery.
pub fn dispatch_consumer_until(
    events: &mut EventRing,
    id: OutputConsumerId,
    consumer: &mut impl EventConsumer,
    mut stop: impl FnMut() -> bool,
) {
    let Some(mut batch) = events.batch(id) else {
        return;
    };
    while !stop() {
        let Some(record) = events.next(&mut batch) else {
            break;
        };
        let submission = consumer.submit(record, &mut events.texts);
        events.submit(id, record.sequence, submission);
    }
}

impl<F> EventConsumer for F
where
    F: FnMut(OutputRecord, &mut TextStore) -> OutputSubmission,
{
    fn submit(&mut self, record: OutputRecord, texts: &mut TextStore) -> OutputSubmission {
        self(record, texts)
    }
}
