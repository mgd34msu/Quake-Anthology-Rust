use crate::primitives::IndexSpan;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotsError {
    Capacity,
    Index,
}

pub struct SlotsLimits {
    pub rows: usize,
    pub width: usize,
    pub bytes: usize,
}

struct Slot {
    bytes: IndexSpan,
    length: u32,
}
impl Slot {
    fn prefix(&self, length: u32) -> std::ops::Range<usize> {
        IndexSpan {
            count: length,
            ..self.bytes
        }
        .range()
    }
}

/// Fixed text rows share one load-sized byte allocation. Raw native bytes and
/// UTF-8 values use the same storage without imposing one wire string policy.
pub struct TextSlots {
    bytes: Box<[u8]>,
    rows: Box<[Slot]>,
}
impl TextSlots {
    pub fn load(
        capacities: impl IntoIterator<Item = usize>,
        limits: SlotsLimits,
    ) -> Result<Self, SlotsError> {
        let mut rows = Vec::new();
        let mut size = 0usize;
        for capacity in capacities {
            if rows.len() == limits.rows || capacity > limits.width {
                return Err(SlotsError::Capacity);
            }
            let first = u32::try_from(size).map_err(|_| SlotsError::Capacity)?;
            let count = u32::try_from(capacity).map_err(|_| SlotsError::Capacity)?;
            size = size.checked_add(capacity).ok_or(SlotsError::Capacity)?;
            if size > limits.bytes || u32::try_from(size).is_err() {
                return Err(SlotsError::Capacity);
            }
            rows.push(Slot {
                bytes: IndexSpan { first, count },
                length: 0,
            });
        }
        Ok(Self {
            bytes: vec![0; size].into_boxed_slice(),
            rows: rows.into_boxed_slice(),
        })
    }
    pub fn len(&self) -> usize {
        self.rows.len()
    }
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
    pub fn capacity(&self, index: usize) -> Option<usize> {
        Some(self.rows.get(index)?.bytes.count as usize)
    }
    pub fn get(&self, index: usize) -> Option<&[u8]> {
        let row = self.rows.get(index)?;
        Some(&self.bytes[row.prefix(row.length)])
    }
    pub fn text(&self, index: usize) -> Option<&str> {
        std::str::from_utf8(self.get(index)?).ok()
    }
    /// Capacity rejection preserves both the previous bytes and their length.
    pub fn set(&mut self, index: usize, text: &[u8]) -> Result<bool, SlotsError> {
        let capacity = self.capacity(index).ok_or(SlotsError::Index)?;
        if text.len() > capacity {
            return Err(SlotsError::Capacity);
        }
        if self.get(index) == Some(text) {
            return Ok(false);
        }
        self.write_parts(index, &[text])?;
        Ok(true)
    }
    /// Event pages may truncate raw parts at their exact native byte capacity.
    /// Returns the stored length and whether any bytes were truncated.
    pub fn write_parts(
        &mut self,
        index: usize,
        parts: &[&[u8]],
    ) -> Result<(usize, bool), SlotsError> {
        let row = self.rows.get_mut(index).ok_or(SlotsError::Index)?;
        let wanted = parts
            .iter()
            .fold(0usize, |size, part| size.saturating_add(part.len()));
        let length = wanted.min(row.bytes.count as usize);
        let output = &mut self.bytes[row.prefix(length as u32)];
        let mut at = 0;
        for part in parts {
            let count = part.len().min(length - at);
            output[at..at + count].copy_from_slice(&part[..count]);
            at += count;
            if at == length {
                break;
            }
        }
        row.length = length as u32;
        Ok((length, length != wanted))
    }
    /// A formatter borrows only this row's reserved region.
    pub fn buffer_mut(&mut self, index: usize) -> Option<&mut [u8]> {
        let row = self.rows.get(index)?;
        Some(&mut self.bytes[row.bytes.range()])
    }
    pub fn set_length(&mut self, index: usize, length: usize) -> Result<(), SlotsError> {
        let row = self.rows.get_mut(index).ok_or(SlotsError::Index)?;
        if length > row.bytes.count as usize {
            return Err(SlotsError::Capacity);
        }
        row.length = length as u32;
        Ok(())
    }
}
