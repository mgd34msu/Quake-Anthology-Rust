use crate::primitives::{EffectEvent, PrintEvent, SoundEvent, TextId};

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
}

pub struct EventRing {
    events: Box<[Option<FrameEvent>]>,
    head: usize,
    len: usize,
    mask: usize,
    overwritten: u64,
}

impl EventRing {
    pub fn load(capacity: usize) -> Result<Self, EventStorageError> {
        if !capacity.is_power_of_two() || capacity > 65536 {
            return Err(EventStorageError::RingCapacity);
        }
        Ok(Self {
            events: vec![None; capacity].into_boxed_slice(),
            head: 0,
            len: 0,
            mask: capacity - 1,
            overwritten: 0,
        })
    }

    pub fn push(&mut self, event: FrameEvent) {
        if self.len == self.events.len() {
            self.head = (self.head + 1) & self.mask;
            self.len -= 1;
            self.overwritten = self.overwritten.saturating_add(1);
        }
        self.events[(self.head + self.len) & self.mask] = Some(event);
        self.len += 1;
    }

    pub fn pop(&mut self) -> Option<FrameEvent> {
        if self.len == 0 {
            return None;
        }
        let event = self.events[self.head].take();
        self.head = (self.head + 1) & self.mask;
        self.len -= 1;
        event
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn overwritten(&self) -> u64 {
        self.overwritten
    }

    pub fn drain(&mut self) -> impl Iterator<Item = FrameEvent> + '_ {
        std::iter::from_fn(|| self.pop())
    }
}

/// Message bytes live here, rather than inside events. Reused rows invalidate
/// old handles, so a delayed HUD never displays somebody else's new message.
pub struct TextStore {
    bytes: Box<[u8]>,
    lengths: Box<[u16]>,
    generations: Box<[u32]>,
    width: usize,
    next: usize,
    overwritten: u64,
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
            width,
            next: 0,
            overwritten: 0,
            truncated: 0,
        })
    }

    pub fn insert(&mut self, text: &[u8]) -> Option<TextId> {
        let rows = self.lengths.len();
        let slot = (0..rows)
            .map(|offset| (self.next + offset) % rows)
            .find(|&slot| self.generations[slot] != u32::MAX)?;
        self.next = (slot + 1) % rows;
        if self.generations[slot] != 0 {
            self.overwritten = self.overwritten.saturating_add(1);
        }
        self.generations[slot] += 1;
        let len = text.len().min(self.width);
        if len != text.len() {
            self.truncated = self.truncated.saturating_add(1);
        }
        self.bytes[slot * self.width..slot * self.width + len].copy_from_slice(&text[..len]);
        self.lengths[slot] = len as u16;
        Some(TextId {
            slot: slot as u16,
            generation: self.generations[slot],
        })
    }

    pub fn get(&self, id: TextId) -> Option<&[u8]> {
        let slot = id.slot as usize;
        if id.generation == 0 || self.generations.get(slot).copied()? != id.generation {
            return None;
        }
        Some(&self.bytes[slot * self.width..slot * self.width + self.lengths[slot] as usize])
    }

    pub fn overwritten(&self) -> u64 {
        self.overwritten
    }
    pub fn truncated(&self) -> u64 {
        self.truncated
    }
}
