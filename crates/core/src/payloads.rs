//! Fixed FIFO storage for typed headers and owned, contiguous byte payloads.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueError {
    Capacity,
    Full,
    PayloadFull,
}

#[derive(Clone, Copy)]
struct Slot<T> {
    header: T,
    start: usize,
    length: usize,
    reserved: usize,
}

pub struct PayloadQueue<T> {
    slots: Box<[Option<Slot<T>>]>,
    bytes: Box<[u8]>,
    head: usize,
    length: usize,
    write: usize,
    used: usize,
}

impl<T: Copy> PayloadQueue<T> {
    pub fn load(slots: usize, bytes: usize) -> Result<Self, QueueError> {
        if !(1..=65536).contains(&slots) || !(1..=64 * 1024 * 1024).contains(&bytes) {
            return Err(QueueError::Capacity);
        }
        Ok(Self {
            slots: vec![None; slots].into_boxed_slice(),
            bytes: vec![0; bytes].into_boxed_slice(),
            head: 0,
            length: 0,
            write: 0,
            used: 0,
        })
    }

    pub fn len(&self) -> usize {
        self.length
    }

    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    pub fn push(
        &mut self,
        header: T,
        payload: &[u8],
        reserve_slots: usize,
    ) -> Result<(), QueueError> {
        if self.length >= self.slots.len().saturating_sub(reserve_slots) {
            return Err(QueueError::Full);
        }
        let length = payload.len();
        if length > self.bytes.len() {
            return Err(QueueError::PayloadFull);
        }
        let padding = if length > self.bytes.len() - self.write {
            self.bytes.len() - self.write
        } else {
            0
        };
        let reserved = padding + length;
        if reserved > self.bytes.len() - self.used {
            return Err(QueueError::PayloadFull);
        }
        let start = if padding == 0 { self.write } else { 0 };
        self.bytes[start..start + length].copy_from_slice(payload);
        self.write = (start + length) % self.bytes.len();
        self.used += reserved;
        self.slots[(self.head + self.length) % self.slots.len()] = Some(Slot {
            header,
            start,
            length,
            reserved,
        });
        self.length += 1;
        Ok(())
    }

    pub fn front(&self) -> Option<(T, &[u8])> {
        self.get(0)
    }

    pub fn get(&self, offset: usize) -> Option<(T, &[u8])> {
        if offset >= self.length {
            return None;
        }
        let slot = self.slots[(self.head + offset) % self.slots.len()].as_ref()?;
        Some((
            slot.header,
            &self.bytes[slot.start..slot.start + slot.length],
        ))
    }

    /// The mutable borrow prevents storage reuse while the payload is read.
    pub fn pop(&mut self) -> Option<(T, &[u8])> {
        let slot = self.slots[self.head].take()?;
        self.head = (self.head + 1) % self.slots.len();
        self.length -= 1;
        self.used -= slot.reserved;
        if self.used == 0 {
            self.write = 0;
        }
        Some((
            slot.header,
            &self.bytes[slot.start..slot.start + slot.length],
        ))
    }

    pub fn clear(&mut self) {
        self.slots.fill(None);
        self.head = 0;
        self.length = 0;
        self.write = 0;
        self.used = 0;
    }
}
