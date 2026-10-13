//! Stable native C-string pointers into load-reserved owned module memory.
use crate::{
    memory::ModuleMemory,
    services::{CallError, ServiceStorage},
};
use qa_core::configstrings::{ConfigLayout, ConfigStrings};
use qa_core::primitives::ModuleId;

pub const Q3_GAME_STATE_ROWS: usize = ConfigLayout::QUAKE3.count;
pub const Q3_GAME_STATE_DATA_BYTES: usize = 16000;
pub const Q3_GAME_STATE_DATA_OFFSET: usize = Q3_GAME_STATE_ROWS * 4;
pub const Q3_GAME_STATE_COUNT_OFFSET: usize = Q3_GAME_STATE_DATA_OFFSET + Q3_GAME_STATE_DATA_BYTES;
pub const Q3_GAME_STATE_BYTES: usize = Q3_GAME_STATE_COUNT_OFFSET + 4;

/// Native CLIENT rows pack into one gameState_t with a reserved empty string.
/// The optional replacement is checked before changing any accepted row.
pub fn q3_game_state_data_count(
    configs: &ConfigStrings,
    replacement: Option<(usize, &[u8])>,
) -> Result<usize, CallError> {
    if configs.len() != Q3_GAME_STATE_ROWS
        || replacement.is_some_and(|(index, text)| index >= Q3_GAME_STATE_ROWS || text.contains(&0))
    {
        return Err(CallError::ConfigString);
    }
    let mut count = 1usize;
    for index in 0..Q3_GAME_STATE_ROWS {
        let text = match replacement {
            Some((changed, text)) if changed == index => text,
            _ => configs.config(index).ok_or(CallError::ConfigString)?.text,
        };
        if !text.is_empty() {
            count = count
                .checked_add(text.len())
                .and_then(|count| count.checked_add(1))
                .filter(|&count| count <= Q3_GAME_STATE_DATA_BYTES)
                .ok_or(CallError::ConfigString)?;
        }
    }
    Ok(count)
}

struct Row {
    address: u64,
    capacity: usize,
    revision: Option<u64>,
}
/// Values remain in ServiceStorage; this owns only the native ABI projection.
pub struct NativeConfigs {
    rows: Box<[Row]>,
    published: Vec<usize>,
    revision: Option<u64>,
}
impl NativeConfigs {
    pub fn byte_length(capacities: &[usize]) -> Result<usize, CallError> {
        if capacities.is_empty() || capacities.len() > 65536 {
            return Err(CallError::Capacity);
        }
        capacities.iter().try_fold(0usize, |size, &capacity| {
            if !(1..=8193).contains(&capacity) {
                return Err(CallError::Capacity);
            }
            size.checked_add(capacity).ok_or(CallError::Capacity)
        })
    }
    pub fn load(mut address: u64, capacities: &[usize]) -> Result<Self, CallError> {
        address
            .checked_add(Self::byte_length(capacities)? as u64)
            .ok_or(CallError::Memory)?;
        let rows = capacities
            .iter()
            .map(|&capacity| {
                let row = Row {
                    address,
                    capacity,
                    revision: None,
                };
                address += capacity as u64;
                row
            })
            .collect();
        Ok(Self {
            rows,
            published: Vec::with_capacity(capacities.len()),
            revision: None,
        })
    }
    pub fn capacity(&self, ordinal: usize) -> Result<usize, CallError> {
        self.rows
            .get(ordinal)
            .map(|row| row.capacity)
            .ok_or(CallError::ConfigString)
    }
    pub fn rebind(&mut self) {
        for &ordinal in &self.published {
            self.rows[ordinal].revision = None;
        }
        self.revision = None;
    }
    fn write(
        row: &mut Row,
        storage: &ServiceStorage,
        module: ModuleId,
        memory: &mut ModuleMemory<'_>,
        ordinal: usize,
    ) -> Result<(), CallError> {
        let (value, revision) = storage.configstring(module, ordinal)?;
        if row.revision != Some(revision) {
            let count = value.len().min(row.capacity - 1);
            memory.write_string(row.address, count + 1, &value[..count])?;
            row.revision = Some(revision);
        }
        Ok(())
    }
    pub fn refresh(
        &mut self,
        storage: &ServiceStorage,
        module: ModuleId,
        memory: &mut ModuleMemory<'_>,
    ) -> Result<(), CallError> {
        let revision = storage.config_revision(module)?;
        if self.revision != Some(revision) {
            for &ordinal in &self.published {
                Self::write(&mut self.rows[ordinal], storage, module, memory, ordinal)?;
            }
            self.revision = Some(revision);
        }
        Ok(())
    }
    pub fn publish(
        &mut self,
        storage: &ServiceStorage,
        module: ModuleId,
        memory: &mut ModuleMemory<'_>,
        ordinal: usize,
    ) -> Result<u64, CallError> {
        self.refresh(storage, module, memory)?;
        let row = self.rows.get_mut(ordinal).ok_or(CallError::ConfigString)?;
        let first = row.revision.is_none();
        Self::write(row, storage, module, memory, ordinal)?;
        if first {
            self.published.push(ordinal);
        }
        Ok(row.address)
    }
}
