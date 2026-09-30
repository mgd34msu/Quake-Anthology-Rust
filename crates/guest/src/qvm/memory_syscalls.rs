//! Memory traps shared by the game, client-game, and UI modules.
//!
//! Port of `src/compat/qvm/memory-syscalls.ts` (memory traps from Quake III
//! Arena's `server/sv_game.c`, `client/cl_cgame.c` and `client/cl_ui.c`;
//! Copyright (C) 1999-2005 Id Software, Inc., GPL-2.0-or-later).
//!
//! All roles share trap numbers 100-102 and return zero from memset/memcpy.
//! The typed strncpy profile returns the original signed destination word
//! (the source returns a host pointer cast to int; its address bits are not
//! reproduced). Undefined overlapping copies and negative counts reject before
//! mutation. `None` means another service owns the trap.

use crate::error::GuestError;

use super::memory::{QvmMemory, QvmWritableView};
use super::syscalls::QvmSyscallRole;

fn reject_overlap(destination: (usize, usize), source: (usize, usize)) -> Result<(), GuestError> {
    let (destination_start, destination_len) = destination;
    let (source_start, source_len) = source;
    if destination_len != 0
        && source_len != 0
        && destination_start < source_start + source_len
        && source_start < destination_start + destination_len
    {
        return Err(GuestError::invalid(
            "Overlapping QVM memcpy/strncpy ranges are unsupported",
        ));
    }
    Ok(())
}

/// Handle memory traps 100-102. Returns `None` when another service owns `trap`.
pub fn qvm_memory_syscall(
    _role: QvmSyscallRole,
    words: &QvmWritableView,
    memory: &QvmMemory,
) -> Result<Option<i32>, GuestError> {
    let trap = words.get_i32(0)?;
    if trap != 100 && trap != 101 && trap != 102 {
        return Ok(None);
    }
    let destination_word = words.get_i32(4)?;
    let source_word = words.get_i32(8)?;
    let count = words.get_i32(12)?;
    if count < 0 {
        return Err(GuestError::invalid("QVM memory trap count is negative"));
    }
    let count = count as usize;
    let destination = memory.span(destination_word, count, 0)?;
    let offset = destination.start();
    if trap == 100 {
        memory.fill_bytes(offset, count, source_word as u8)?;
        return Ok(Some(0));
    }
    if trap == 101 {
        let source = memory.span(source_word, count, 0)?;
        reject_overlap((destination.start(), destination.len()), (source.start(), source.len()))?;
        let bytes = source.to_vec();
        memory.write_bytes(offset, &bytes)?;
        return Ok(Some(0));
    }
    let source = memory
        .pointer(source_word)?
        .ok_or_else(|| GuestError::invalid("QVM strncpy requires a nonnull source pointer"))?;
    let head = source.subspan(0, count.min(source.len()))?;
    let terminator = head.index_of(0);
    let copied_length = terminator.unwrap_or(count);
    let consumed_length = terminator.map_or(count, |end| end + 1);
    let consumed = memory.span(source_word, consumed_length, 0)?;
    reject_overlap(
        (destination.start(), destination.len()),
        (consumed.start(), consumed.len()),
    )?;
    let copied = source.subspan(0, copied_length)?.to_vec();
    memory.write_bytes(offset, &copied)?;
    memory.fill_bytes(offset + copied_length, count - copied_length, 0)?;
    Ok(Some(destination_word))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(trap: i32, destination: i32, source: i32, count: i32) -> (QvmMemory, QvmWritableView) {
        let memory = QvmMemory::new(vec![0; 256]).unwrap();
        let words = memory.data_view(224, 32).unwrap();
        words.set_i32(0, trap).unwrap();
        words.set_i32(4, destination).unwrap();
        words.set_i32(8, source).unwrap();
        words.set_i32(12, count).unwrap();
        (memory, words)
    }

    #[test]
    fn memset_fills_and_returns_zero() {
        let (memory, words) = fixture(100, 16, 0xAB, 8);
        assert_eq!(
            qvm_memory_syscall(QvmSyscallRole::Game, &words, &memory).unwrap(),
            Some(0)
        );
        assert_eq!(memory.read_bytes(16, 8).unwrap(), vec![0xAB; 8]);
    }

    #[test]
    fn memcpy_rejects_overlap_before_mutation() {
        let (memory, words) = fixture(101, 20, 16, 8);
        memory.write_bytes(16, &[1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
        assert!(qvm_memory_syscall(QvmSyscallRole::Cgame, &words, &memory).is_err());
        assert_eq!(memory.read_bytes(20, 4).unwrap(), vec![5, 6, 7, 8]);
        let memory = QvmMemory::new(vec![0; 256]).unwrap();
        memory.write_bytes(16, &[1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
        let words = memory.data_view(224, 32).unwrap();
        words.set_i32(0, 101).unwrap();
        words.set_i32(4, 64).unwrap();
        words.set_i32(8, 16).unwrap();
        words.set_i32(12, 8).unwrap();
        assert_eq!(
            qvm_memory_syscall(QvmSyscallRole::Ui, &words, &memory).unwrap(),
            Some(0)
        );
        assert_eq!(memory.read_bytes(64, 8).unwrap(), vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn strncpy_returns_destination_and_pads() {
        let memory = QvmMemory::new(vec![0; 256]).unwrap();
        memory.write_bytes(16, b"hi\0junk").unwrap();
        let words = memory.data_view(224, 32).unwrap();
        words.set_i32(0, 102).unwrap();
        words.set_i32(4, 64).unwrap();
        words.set_i32(8, 16).unwrap();
        words.set_i32(12, 6).unwrap();
        assert_eq!(
            qvm_memory_syscall(QvmSyscallRole::Game, &words, &memory).unwrap(),
            Some(64)
        );
        assert_eq!(memory.read_bytes(64, 6).unwrap(), vec![b'h', b'i', 0, 0, 0, 0]);
    }

    #[test]
    fn foreign_traps_and_negative_counts() {
        let (memory, words) = fixture(42, 16, 16, 4);
        assert_eq!(qvm_memory_syscall(QvmSyscallRole::Game, &words, &memory).unwrap(), None);
        let (memory, words) = fixture(100, 16, 1, -1);
        assert!(qvm_memory_syscall(QvmSyscallRole::Game, &words, &memory).is_err());
        let (memory, words) = fixture(102, 16, 0, 4);
        assert!(qvm_memory_syscall(QvmSyscallRole::Game, &words, &memory).is_err());
    }
}
