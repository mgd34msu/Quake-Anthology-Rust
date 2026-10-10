//! Native cvar layouts are projections of the console's one registry.
use crate::{memory::ModuleMemory, services::CallError};
use qa_console::{
    cvars::{Cvars, View},
    numbers::integer,
    text::MAX_TEXT,
};

const TEXT_BYTES: usize = MAX_TEXT + 1;
const HEADER_BYTES: usize = 56;
const NAME: usize = HEADER_BYTES;
const VALUE: usize = NAME + TEXT_BYTES;
const LATCH: usize = VALUE + TEXT_BYTES;
const STRIDE: usize = (LATCH + TEXT_BYTES).div_ceil(8) * 8;

struct Row {
    view: View,
    update_generation: u64,
    extra_flags: u32,
}

/// Fixed native cvar_t slots and their C strings are reserved with the image.
/// This owns ABI bindings only; values, defaults and policy remain in Cvars.
pub struct NativeCvars {
    address: u64,
    capacity: usize,
    count_modified: bool,
    rows: Vec<Row>,
    revision: Option<u64>,
    pub updates: u64,
    pub capacity_drops: u64,
}
impl NativeCvars {
    pub fn byte_length(capacity: usize) -> Option<usize> {
        capacity.checked_mul(STRIDE)
    }
    pub fn load(address: u64, capacity: usize, count_modified: bool) -> Self {
        Self {
            address,
            capacity,
            count_modified,
            rows: Vec::with_capacity(capacity),
            revision: None,
            updates: 0,
            capacity_drops: 0,
        }
    }
    fn address(&self, index: usize) -> Result<u64, CallError> {
        self.address
            .checked_add((index * STRIDE) as u64)
            .ok_or(CallError::Memory)
    }
    pub fn publish(
        &mut self,
        cvars: &Cvars,
        memory: &mut ModuleMemory<'_>,
        view: View,
        native_flags: u32,
    ) -> Result<u64, CallError> {
        self.refresh(cvars, memory)?;
        let index = if let Some(index) = self
            .rows
            .iter()
            .position(|row| row.view.binding_id() == view.binding_id())
        {
            let flags = self.rows[index].extra_flags | (native_flags & !31);
            if flags != self.rows[index].extra_flags {
                self.rows[index].extra_flags = flags;
                self.refresh_row(cvars, memory, index, false)?;
            }
            index
        } else {
            if self.rows.len() == self.capacity {
                self.capacity_drops = self.capacity_drops.saturating_add(1);
                return Err(CallError::Capacity);
            }
            let index = self.rows.len();
            let address = self.address(index)?;
            memory.read(address, STRIDE)?;
            write_text(memory, address + NAME as u64, cvars.name(view))?;
            let mut header = [0u8; HEADER_BYTES];
            header[..8].copy_from_slice(&(address + NAME as u64).to_le_bytes());
            header[8..16].copy_from_slice(&(address + VALUE as u64).to_le_bytes());
            if index != 0 {
                header[40..48].copy_from_slice(&self.address(index - 1)?.to_le_bytes());
            }
            memory.write(address, &header)?;
            self.rows.push(Row {
                view,
                update_generation: cvars.view_update_generation(view),
                extra_flags: native_flags & !31,
            });
            self.refresh_row(cvars, memory, index, true)?;
            index
        };
        self.address(index)
    }
    pub fn refresh(
        &mut self,
        cvars: &Cvars,
        memory: &mut ModuleMemory<'_>,
    ) -> Result<(), CallError> {
        if self.revision == Some(cvars.revision()) {
            return Ok(());
        }
        for index in 0..self.rows.len() {
            let row = &self.rows[index];
            if row.update_generation != cvars.view_update_generation(row.view) {
                self.refresh_row(cvars, memory, index, false)?;
            }
        }
        self.revision = Some(cvars.revision());
        Ok(())
    }
    fn refresh_row(
        &mut self,
        cvars: &Cvars,
        memory: &mut ModuleMemory<'_>,
        index: usize,
        initial: bool,
    ) -> Result<(), CallError> {
        let address = self.address(index)?;
        let row = &mut self.rows[index];
        let value = cvars.read(row.view).map_err(|_| CallError::Cvar)?;
        let value = value.as_str();
        let changed = initial || memory.cstring(address + VALUE as u64)? != value.as_bytes();
        if changed {
            write_text(memory, address + VALUE as u64, value)?;
            let number = cvars.numeric(row.view).map_err(|_| CallError::Cvar)?;
            memory.write(address + 32, &number.to_le_bytes())?;
            if self.count_modified {
                memory.write_word(address + 48, integer(value))?;
            }
            let modified = if !self.count_modified || initial {
                1
            } else {
                let next = memory.read_word(address + 28)?.wrapping_add(1);
                if next == 0 { 1 } else { next }
            };
            memory.write_word(address + 28, modified)?;
        }
        let flags = native_flags(cvars.flags(row.view)) | row.extra_flags;
        memory.write(address + 24, &flags.to_le_bytes())?;
        let latch = cvars.latched(row.view).map_err(|_| CallError::Cvar)?;
        let latch_address = if let Some(latch) = latch {
            write_text(memory, address + LATCH as u64, latch.as_str())?;
            address + LATCH as u64
        } else {
            0
        };
        memory.write(address + 16, &latch_address.to_le_bytes())?;
        row.update_generation = cvars.view_update_generation(row.view);
        self.updates = self.updates.saturating_add(1);
        Ok(())
    }
}

pub fn common_flags(native: u32) -> u32 {
    (native & 7) | ((native & 8) << 1) | ((native & 16) << 1)
}
fn native_flags(common: u32) -> u32 {
    (common & 7) | (u32::from(common & (16 | 64) != 0) << 3) | ((common & 32) >> 1)
}
fn write_text(memory: &mut ModuleMemory<'_>, address: u64, text: &str) -> Result<(), CallError> {
    if text.len() > MAX_TEXT || text.as_bytes().contains(&0) {
        return Err(CallError::Text);
    }
    memory.write(address, text.as_bytes())?;
    memory.write(address + text.len() as u64, &[0])?;
    Ok(())
}
