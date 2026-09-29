//! Guest client-state view and shared syscall-bridge foundation.
//!
//! Provenance: `src/compat/qvm/client-state.ts` for [`ClientState`],
//! [`GameStateRecord`], and [`WireUserCommand`]. The [`SyscallMemory`],
//! [`HostCall`], [`QvmRole`], [`CallKind`], and [`AbiProfile`] types are local
//! mirrors of `src/compat/qvm/memory.ts` (`QvmMemory`), `syscalls.ts`
//! (`QvmHostCall`), and `src/contracts/execution.ts` (`QvmAbiProfile`); the
//! canonical qvm memory/interpreter modules are owned by other workers, so
//! every file in this port reuses these mirrors via
//! `super::client_state::...` instead.

use qa_core::math::Vec3;

use crate::error::GuestError;

/// Selected QVM ABI: modern (`q3-modern`) or legacy (`q3-1.16n-base`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AbiProfile {
    /// Modern Quake III ABI.
    #[default]
    Modern,
    /// Legacy 1.16n/1.17 ABI.
    Legacy,
}

impl AbiProfile {
    /// Whether this is the modern ABI.
    #[must_use]
    pub const fn is_modern(self) -> bool {
        matches!(self, Self::Modern)
    }
}

/// Guest module role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmRole {
    /// Server game module.
    Qagame,
    /// Client game module.
    Cgame,
    /// User-interface module.
    Ui,
}

/// Engine trap versus raw extension call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallKind {
    /// Classified engine trap.
    Engine,
    /// Unclassified raw trap number.
    Extension,
}

/// One decoded host call: trap words plus routing metadata.
///
/// `words[0]` is the trap number; arguments follow at `words[1..]`, matching
/// the donor's `DataView` word indexing (`words.getInt32(index * 4, true)`).
#[derive(Debug, Clone)]
pub struct HostCall {
    /// Engine or extension classification.
    pub kind: CallKind,
    /// Calling module role.
    pub role: QvmRole,
    /// Trap number (already classified for engine calls).
    pub code: i32,
    /// Raw trap words, including the trap number at index 0.
    pub words: Vec<i32>,
    /// Selected ABI profile.
    pub abi_profile: AbiProfile,
}

impl HostCall {
    /// Build an engine call with the given trap number and argument words.
    #[must_use]
    pub fn engine(role: QvmRole, code: i32, args: &[i32], abi_profile: AbiProfile) -> Self {
        let mut words = Vec::with_capacity(args.len() + 1);
        words.push(code);
        words.extend_from_slice(args);
        Self {
            kind: CallKind::Engine,
            role,
            code,
            words,
            abi_profile,
        }
    }

    /// Build an extension call with the given trap number and argument words.
    #[must_use]
    pub fn extension(role: QvmRole, code: i32, args: &[i32], abi_profile: AbiProfile) -> Self {
        let mut words = Vec::with_capacity(args.len() + 1);
        words.push(code);
        words.extend_from_slice(args);
        Self {
            kind: CallKind::Extension,
            role,
            code,
            words,
            abi_profile,
        }
    }

    /// Read the `index`-th word as a signed integer.
    pub fn int(&self, index: usize) -> Result<i32, GuestError> {
        self.words.get(index).copied().ok_or_else(|| {
            GuestError::invalid(format!("Host call has no word at index {index}"))
        })
    }

    /// Read the `index`-th word as a little-endian float.
    pub fn float(&self, index: usize) -> Result<f32, GuestError> {
        Ok(f32::from_bits(self.int(index)? as u32))
    }
}

/// Minimal guest-memory view with QVM pointer-masking semantics.
///
/// The allocation is a power of two; nonzero guest pointers mask to a base
/// offset (`word & (len - 1)`, matching the donor's 32-bit masking including
/// negative words) and subsequent bytes advance without wrapping. All
/// out-of-range access returns [`GuestError`]; nothing panics on guest input.
#[derive(Debug)]
pub struct SyscallMemory {
    bytes: Vec<u8>,
    mask: usize,
}

impl SyscallMemory {
    /// Largest supported allocation: 2^30 bytes, matching the donor.
    pub const MAX_BYTES: usize = 0x4000_0000;

    /// Allocate a zeroed guest memory of `byte_len` bytes.
    pub fn new(byte_len: usize) -> Result<Self, GuestError> {
        Self::from_bytes(vec![0u8; byte_len])
    }

    /// Wrap existing bytes as guest memory.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, GuestError> {
        let len = bytes.len();
        if len == 0 || len > Self::MAX_BYTES || !len.is_power_of_two() {
            return Err(GuestError::invalid(
                "QVM memory allocation must be a nonzero power of two at most 2^30 bytes",
            ));
        }
        Ok(Self {
            bytes,
            mask: len - 1,
        })
    }

    /// Allocation length in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Whether the allocation is empty (never true for valid memories).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Raw read access to the backing bytes.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    /// Resolve a guest pointer word to an absolute base offset.
    ///
    /// Word 0 is null; any other word masks into the allocation.
    #[must_use]
    pub fn pointer(&self, word: i32) -> Option<usize> {
        if word == 0 {
            None
        } else {
            Some((word as u32 as usize) & self.mask)
        }
    }

    /// Resolve a guest span: masked base plus `relative` offset, `len` bytes.
    pub fn span(&self, word: i32, len: usize, relative: isize) -> Result<std::ops::Range<usize>, GuestError> {
        let base = self.pointer(word).ok_or_else(|| {
            GuestError::invalid("QVM memory span requires a nonnull pointer")
        })?;
        let start = base as isize + relative;
        let end = start + len as isize;
        if relative < 0 && start < 0 || start < 0 || end < start || end as usize > self.bytes.len() {
            return Err(GuestError::memory_fault(
                "span-overflow",
                word as u32 as u64,
                len,
                "read",
                "QVM memory span exceeds allocation or has an invalid length",
            ));
        }
        Ok(start as usize..end as usize)
    }

    fn check(&self, offset: usize, len: usize, access: &'static str) -> Result<std::ops::Range<usize>, GuestError> {
        let end = offset.saturating_add(len);
        if end > self.bytes.len() || end < offset {
            return Err(GuestError::memory_fault(
                "range-overflow",
                offset as u64,
                len,
                access,
                "QVM memory range exceeds allocation",
            ));
        }
        Ok(offset..end)
    }

    /// Read one byte at an absolute offset.
    pub fn get(&self, offset: usize) -> Result<u8, GuestError> {
        self.check(offset, 1, "read")?;
        Ok(self.bytes[offset])
    }

    /// Write one byte at an absolute offset.
    pub fn set(&mut self, offset: usize, value: u8) -> Result<(), GuestError> {
        self.check(offset, 1, "write")?;
        self.bytes[offset] = value;
        Ok(())
    }

    /// Read a little-endian `i32` at an absolute offset.
    pub fn read_i32(&self, offset: usize) -> Result<i32, GuestError> {
        self.check(offset, 4, "read")?;
        Ok(i32::from_le_bytes(self.bytes[offset..offset + 4].try_into().expect("checked range")))
    }

    /// Write a little-endian `i32` at an absolute offset.
    pub fn write_i32(&mut self, offset: usize, value: i32) -> Result<(), GuestError> {
        self.check(offset, 4, "write")?;
        self.bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    /// Read a little-endian `u16` at an absolute offset.
    pub fn read_u16(&self, offset: usize) -> Result<u16, GuestError> {
        self.check(offset, 2, "read")?;
        Ok(u16::from_le_bytes(self.bytes[offset..offset + 2].try_into().expect("checked range")))
    }

    /// Write a little-endian `u16` at an absolute offset.
    pub fn write_u16(&mut self, offset: usize, value: u16) -> Result<(), GuestError> {
        self.check(offset, 2, "write")?;
        self.bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    /// Read a little-endian `f32` at an absolute offset.
    pub fn read_f32(&self, offset: usize) -> Result<f32, GuestError> {
        self.check(offset, 4, "read")?;
        Ok(f32::from_le_bytes(self.bytes[offset..offset + 4].try_into().expect("checked range")))
    }

    /// Write a little-endian `f32` at an absolute offset.
    pub fn write_f32(&mut self, offset: usize, value: f32) -> Result<(), GuestError> {
        self.check(offset, 4, "write")?;
        self.bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    /// Read a signed byte at an absolute offset.
    pub fn read_i8(&self, offset: usize) -> Result<i8, GuestError> {
        Ok(self.get(offset)? as i8)
    }

    /// Write a signed byte at an absolute offset.
    pub fn write_i8(&mut self, offset: usize, value: i8) -> Result<(), GuestError> {
        self.set(offset, value as u8)
    }

    /// Read three little-endian floats at an absolute offset.
    pub fn read_vec3(&self, offset: usize) -> Result<Vec3, GuestError> {
        Ok(Vec3 {
            x: self.read_f32(offset)?,
            y: self.read_f32(offset + 4)?,
            z: self.read_f32(offset + 8)?,
        })
    }

    /// Write three little-endian floats at an absolute offset.
    pub fn write_vec3(&mut self, offset: usize, value: &Vec3) -> Result<(), GuestError> {
        self.write_f32(offset, value.x)?;
        self.write_f32(offset + 4, value.y)?;
        self.write_f32(offset + 8, value.z)
    }

    /// Read a vector from a guest pointer word.
    pub fn read_vec3_ptr(&self, word: i32) -> Result<Vec3, GuestError> {
        let base = self.pointer(word).ok_or_else(|| {
            GuestError::invalid("QVM vector requires a nonnull pointer")
        })?;
        self.read_vec3(base)
    }

    /// Borrow a byte range at an absolute offset.
    pub fn read_bytes(&self, offset: usize, len: usize) -> Result<&[u8], GuestError> {
        let range = self.check(offset, len, "read")?;
        Ok(&self.bytes[range])
    }

    /// Copy bytes to an absolute offset.
    pub fn write_bytes(&mut self, offset: usize, bytes: &[u8]) -> Result<(), GuestError> {
        let range = self.check(offset, bytes.len(), "write")?;
        self.bytes[range].copy_from_slice(bytes);
        Ok(())
    }

    /// Fill a range at an absolute offset with `value`.
    pub fn fill(&mut self, offset: usize, len: usize, value: u8) -> Result<(), GuestError> {
        let range = self.check(offset, len, "write")?;
        self.bytes[range].fill(value);
        Ok(())
    }

    /// Read a NUL-terminated byte string from a guest pointer.
    ///
    /// Bytes map to Latin-1 scalar values, matching the donor's
    /// `String.fromCharCode` decoding byte for byte.
    pub fn read_string(&self, word: i32) -> Result<String, GuestError> {
        let base = self.pointer(word).ok_or_else(|| {
            GuestError::invalid("QVM string requires a nonnull pointer")
        })?;
        let tail = &self.bytes[base..];
        let end = tail.iter().position(|byte| *byte == 0).ok_or_else(|| {
            GuestError::invalid("QVM string has no terminator before the allocation ends")
        })?;
        Ok(tail[..end].iter().map(|byte| char::from(*byte)).collect())
    }

    /// Write `text` with `Q_strncpyz` semantics: at most `capacity - 1` bytes,
    /// always NUL-terminated, zero-padded through `capacity`.
    ///
    /// Each character contributes its low byte, matching the donor's
    /// `charCodeAt & 255` encoding.
    pub fn write_string(&mut self, word: i32, text: &str, capacity: usize) -> Result<(), GuestError> {
        if capacity < 1 {
            return Err(GuestError::invalid("Q_strncpyz: destsize < 1"));
        }
        let base = self.pointer(word).ok_or_else(|| {
            GuestError::invalid("Q_strncpyz: NULL dest")
        })?;
        if capacity > self.bytes.len() - base {
            return Err(GuestError::memory_fault(
                "string-overflow",
                word as u32 as u64,
                capacity,
                "write",
                "Q_strncpyz destination exceeds QVM allocation",
            ));
        }
        let mut index = 0usize;
        for ch in text.chars() {
            if index + 1 >= capacity {
                break;
            }
            #[allow(clippy::char_lit_as_u8)]
            self.bytes[base + index] = (ch as u32 & 0xFF) as u8;
            index += 1;
        }
        self.bytes[base + index..base + capacity].fill(0);
        Ok(())
    }

    /// Raw bounded copy plus NUL: null pointers and bad capacities are errors
    /// without the donor's fatal-comment path.
    pub fn write_bounded_string(&mut self, word: i32, text: &str, capacity: usize) -> Result<(), GuestError> {
        let base = self.pointer(word).ok_or_else(|| {
            GuestError::invalid("Bounded string copy exceeds QVM allocation")
        })?;
        if capacity < 1 || capacity > self.bytes.len() - base {
            return Err(GuestError::invalid("Bounded string copy exceeds QVM allocation"));
        }
        self.write_string(word, text, capacity)
    }
}

/// Wire user command shared by client and bot-library traps.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WireUserCommand {
    /// Server time in milliseconds.
    pub server_time: i32,
    /// View angles as fixed-point words.
    pub angles: [i32; 3],
    /// Button bits.
    pub buttons: i32,
    /// Selected weapon.
    pub weapon: u8,
    /// Forward move offset.
    pub forwardmove: i8,
    /// Right move offset.
    pub rightmove: i8,
    /// Up move offset.
    pub upmove: i8,
}

/// Detached game-state record: configstring offsets plus string data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameStateRecord {
    /// Per-configstring offsets into `string_data`.
    pub string_offsets: Vec<i32>,
    /// Packed configstring bytes.
    pub string_data: Vec<u8>,
    /// String data byte count.
    pub data_count: i32,
}

/// Retained engine state that client VM services depend on.
pub trait ClientState {
    /// Connection generation token.
    fn generation(&self) -> u64;
    /// Last received server-message sequence.
    fn server_message_sequence(&self) -> i32;
    /// Last executed server-command sequence.
    fn last_executed_server_command(&self) -> i32;
    /// Local client number.
    fn client_number(&self) -> i32;
    /// Ping recorded for a retained snapshot, if any.
    fn snapshot_ping(&self, number: i32) -> Option<i32>;
    /// Configstring value at a canonical index, if present.
    fn game_state_get(&self, index: usize) -> Option<String>;
    /// Detached copy of the full game-state record.
    fn game_state_record(&self) -> GameStateRecord;
    /// Current outgoing user-command number.
    fn commands_current_number(&self) -> i32;
    /// Retained user command by number, if any.
    fn commands_read(&self, number: i32) -> Option<WireUserCommand>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory() -> SyscallMemory {
        SyscallMemory::new(1024).unwrap()
    }

    #[test]
    fn rejects_non_power_of_two() {
        assert!(SyscallMemory::new(0).is_err());
        assert!(SyscallMemory::new(1000).is_err());
        assert!(SyscallMemory::new(1024).is_ok());
    }

    #[test]
    fn null_pointer_is_none() {
        let memory = memory();
        assert_eq!(memory.pointer(0), None);
        assert_eq!(memory.pointer(1024), Some(0));
        assert_eq!(memory.pointer(-1), Some(1023));
    }

    #[test]
    fn span_rejects_overflow() {
        let memory = memory();
        assert!(memory.span(1000, 24, 0).is_ok());
        assert!(memory.span(1000, 25, 0).is_err());
        assert!(memory.span(0, 4, 0).is_err());
    }

    #[test]
    fn string_round_trip_with_padding() {
        let mut memory = memory();
        memory.write_string(64, "hi", 8).unwrap();
        assert_eq!(memory.read_string(64).unwrap(), "hi");
        assert_eq!(memory.read_bytes(64, 8).unwrap(), &[b'h', b'i', 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn string_truncates_and_terminates() {
        let mut memory = memory();
        memory.write_string(64, "abcdef", 4).unwrap();
        assert_eq!(memory.read_string(64).unwrap(), "abc");
        assert_eq!(memory.get(67).unwrap(), 0);
    }

    #[test]
    fn string_errors() {
        let mut memory = memory();
        assert!(memory.write_string(0, "x", 4).is_err());
        assert!(memory.write_string(64, "x", 0).is_err());
        assert!(memory.write_string(1024, "toolongcapacity", 2048).is_err());
        assert!(memory.read_string(0).is_err());
        memory.fill(0, 1024, 0x41).unwrap();
        assert!(memory.read_string(64).is_err());
    }

    #[test]
    fn scalars_and_vectors() {
        let mut memory = memory();
        memory.write_i32(100, -123456).unwrap();
        assert_eq!(memory.read_i32(100).unwrap(), -123456);
        memory.write_f32(200, 1.5).unwrap();
        assert_eq!(memory.read_f32(200).unwrap(), 1.5);
        memory.write_u16(300, 0xBEEF).unwrap();
        assert_eq!(memory.read_u16(300).unwrap(), 0xBEEF);
        let vector = Vec3 { x: 1.0, y: -2.0, z: 3.5 };
        memory.write_vec3(400, &vector).unwrap();
        assert_eq!(memory.read_vec3(400).unwrap(), vector);
        assert_eq!(memory.read_vec3_ptr(400).unwrap(), vector);
        assert!(memory.read_i32(1024).is_err());
    }

    #[test]
    fn host_call_words() {
        let call = HostCall::engine(QvmRole::Cgame, 55, &[1, 2], AbiProfile::Modern);
        assert_eq!(call.int(0).unwrap(), 55);
        assert_eq!(call.int(2).unwrap(), 2);
        assert!(call.int(3).is_err());
        let call = HostCall::engine(QvmRole::Cgame, 56, &[0, 1.0f32.to_bits() as i32], AbiProfile::Modern);
        assert_eq!(call.float(2).unwrap(), 1.0);
    }

    #[test]
    fn latin1_bytes_round_trip() {
        let mut memory = memory();
        memory.write_string(64, "caf\u{e9}", 8).unwrap();
        assert_eq!(memory.read_bytes(64, 5).unwrap(), &[b'c', b'a', b'f', 0xE9, 0]);
        assert_eq!(memory.read_string(64).unwrap(), "caf\u{e9}");
    }
}
