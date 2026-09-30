//! Quake III base/game: memory.
//!
//! Donor provenance: `src/content/q3/base/game/memory.ts`.

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::mirrors_game_items::*;

// ---------------------------------------------------------------------------
// memory.ts: g_mem.c
// ---------------------------------------------------------------------------

/// Game memory pool size (`GAME_MEMORY_BYTES`).
pub const GAME_MEMORY_BYTES: usize = 256 * 1024;

/// Allocation handle into the game pool (`GameMemoryAllocation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GameMemoryAllocation {
    offset: usize,
    length: usize,
}

impl GameMemoryAllocation {
    /// Offset in the pool.
    #[must_use]
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Allocation length.
    #[must_use]
    pub fn length(&self) -> usize {
        self.length
    }

    /// Read a NUL-terminated string, reaching into retained pool tails.
    pub fn read_string(&self, memory: &GameMemory) -> Q3GameItemsResult<String> {
        if self.offset > memory.pool.len() {
            return Err(range("game allocation belongs to another pool"));
        }
        let remaining = &memory.pool[self.offset..];
        let end = remaining
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(|| range("game string reads beyond the source memory pool"))?;
        Ok(remaining[..end]
            .iter()
            .map(|byte| char::from_u32(u32::from(*byte)).expect("byte is valid scalar"))
            .collect())
    }

    /// Write a NUL-terminated string into the allocation.
    pub fn write_string(&self, memory: &mut GameMemory, value: &str) -> Q3GameItemsResult<()> {
        let units: Vec<u16> = value.encode_utf16().collect();
        if units.len() + 1 > self.length {
            return Err(range("game string exceeds its source allocation"));
        }
        if self.offset + units.len() + 1 > memory.pool.len() {
            return Err(range("game allocation belongs to another pool"));
        }
        for (index, unit) in units.iter().enumerate() {
            if *unit == 0 || *unit > 255 {
                return Err(range("game strings require non-NUL source bytes"));
            }
            memory.pool[self.offset + index] = *unit as u8;
        }
        memory.pool[self.offset + units.len()] = 0;
        Ok(())
    }
}

/// Captured allocation bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapturedAllocation {
    /// Offset in the pool.
    pub offset: usize,
    /// Allocation length.
    pub length: usize,
}

/// Game memory save image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameMemorySave {
    /// Pool bytes.
    pub pool: Vec<u8>,
    /// Allocation point.
    pub alloc_point: usize,
}

/// Module-owned static storage (`GameMemory`).
pub struct GameMemory {
    pool: Vec<u8>,
    alloc_point: usize,
    debug_integer: Box<dyn FnMut() -> i32>,
    print: Box<dyn FnMut(String)>,
}

impl GameMemory {
    /// Memory with debug and print hooks.
    pub fn new(debug_integer: Box<dyn FnMut() -> i32>, print: Box<dyn FnMut(String)>) -> GameMemory {
        GameMemory {
            pool: vec![0; GAME_MEMORY_BYTES],
            alloc_point: 0,
            debug_integer,
            print,
        }
    }

    /// Allocated bytes.
    #[must_use]
    pub fn allocated_bytes(&self) -> usize {
        self.alloc_point
    }

    /// Allocate pool storage (`G_Alloc`).
    pub fn allocate(&mut self, size: usize) -> Q3GameItemsResult<GameMemoryAllocation> {
        if size > i32::MAX as usize {
            return Err(range("G_Alloc requires a nonnegative source int size"));
        }
        let aligned = size.wrapping_add(31) & !31;
        if (self.debug_integer)() != 0 {
            let left = (GAME_MEMORY_BYTES as i64 - self.alloc_point as i64 - aligned as i64) as i32;
            (self.print)(format!("G_Alloc of {size} bytes ({left} left)\n"));
        }
        if self.alloc_point + size > GAME_MEMORY_BYTES {
            return Err(drop_error(format!("G_Alloc: failed on allocation of {size} bytes\n")));
        }
        let allocation = GameMemoryAllocation {
            offset: self.alloc_point,
            length: size,
        };
        self.alloc_point = self.alloc_point.wrapping_add(aligned);
        Ok(allocation)
    }

    /// Rewind the pool (`G_InitMemory`).
    pub fn initialize(&mut self) {
        self.alloc_point = 0;
    }

    /// Print pool status.
    pub fn status(&mut self) {
        (self.print)(format!(
            "Game memory status: {} out of {GAME_MEMORY_BYTES} bytes allocated\n",
            self.alloc_point
        ));
    }

    /// Capture allocation bounds.
    pub fn capture_allocation(&self, allocation: &GameMemoryAllocation) -> Q3GameItemsResult<CapturedAllocation> {
        if allocation.offset + allocation.length > self.pool.len() {
            return Err(invalid("game allocation belongs to another pool"));
        }
        Ok(CapturedAllocation {
            offset: allocation.offset,
            length: allocation.length,
        })
    }

    /// Restore allocation bounds.
    pub fn restore_allocation(&self, captured: &CapturedAllocation) -> Q3GameItemsResult<GameMemoryAllocation> {
        if captured.offset > self.pool.len() || captured.length > self.pool.len() - captured.offset {
            return Err(range("allocation outside game pool"));
        }
        Ok(GameMemoryAllocation {
            offset: captured.offset,
            length: captured.length,
        })
    }

    /// Capture the save image.
    #[must_use]
    pub fn capture_save_state(&self) -> GameMemorySave {
        GameMemorySave {
            pool: self.pool.clone(),
            alloc_point: self.alloc_point,
        }
    }

    /// Restore the save image.
    pub fn restore_save_state(&mut self, save: &GameMemorySave) -> Q3GameItemsResult<()> {
        if save.pool.len() != GAME_MEMORY_BYTES
            || save.alloc_point > GAME_MEMORY_BYTES
            || !save.alloc_point.is_multiple_of(32)
        {
            return Err(range("invalid module memory extent"));
        }
        self.pool.copy_from_slice(&save.pool);
        self.alloc_point = save.alloc_point;
        Ok(())
    }
}
