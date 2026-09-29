//! Guest integer registers, flags, segments, and processor-state assembly.
//!
//! Donor: `src/guest/core/registers.ts`. Sixteen physical 64-bit slots with the
//! legacy low/high byte aliases; a 32-bit write clears the upper half in
//! x86-64 mode. The donor's frozen "managed" views are inherent to Rust's
//! ownership model, so only one implementation exists.

use crate::core::contracts::{GuestArchitecture, GuestFlag, GuestIntegerWidth, GuestRegister};
use crate::error::GuestError;

/// Physical 64-bit register slots, including legacy byte aliases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegerRegisterFile {
    architecture: GuestArchitecture,
    slots: [u64; 16],
}

impl IntegerRegisterFile {
    /// Zeroed register file for `architecture`.
    #[must_use]
    pub const fn new(architecture: GuestArchitecture) -> Self {
        Self {
            architecture,
            slots: [0; 16],
        }
    }

    /// Configured architecture.
    #[must_use]
    pub const fn architecture(&self) -> GuestArchitecture {
        self.architecture
    }

    /// Read `width` bits of `register`. `high_byte` selects AH/CH/DH/BH.
    pub fn read(
        &self,
        register: GuestRegister,
        width: GuestIntegerWidth,
        high_byte: bool,
    ) -> Result<u64, GuestError> {
        self.check(register, width, high_byte)?;
        let slot = self.slots[register.index()];
        Ok(match width {
            GuestIntegerWidth::B64 => slot,
            GuestIntegerWidth::B32 => slot & 0xffff_ffff,
            GuestIntegerWidth::B16 => slot & 0xffff,
            GuestIntegerWidth::B8 if high_byte => (slot >> 8) & 0xff,
            GuestIntegerWidth::B8 => slot & 0xff,
        })
    }

    /// Write `width` bits of `register`. A 32-bit write clears the upper
    /// half; `high_byte` selects AH/CH/DH/BH.
    pub fn write(
        &mut self,
        register: GuestRegister,
        width: GuestIntegerWidth,
        value: u64,
        high_byte: bool,
    ) -> Result<(), GuestError> {
        self.check(register, width, high_byte)?;
        let slot = &mut self.slots[register.index()];
        match width {
            GuestIntegerWidth::B64 => *slot = value,
            GuestIntegerWidth::B32 => *slot = value & 0xffff_ffff,
            GuestIntegerWidth::B16 => *slot = (*slot & !0xffff) | (value & 0xffff),
            GuestIntegerWidth::B8 if high_byte => {
                *slot = (*slot & !0xff00) | ((value & 0xff) << 8);
            }
            GuestIntegerWidth::B8 => *slot = (*slot & !0xff) | (value & 0xff),
        }
        Ok(())
    }

    /// Snapshot the raw slots (64 bytes on i386, 128 on x86-64).
    #[must_use]
    pub fn checkpoint(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.snapshot_len());
        let count = self.slot_count();
        for slot in self.slots.iter().take(count) {
            bytes.extend_from_slice(&slot.to_le_bytes());
        }
        bytes
    }

    /// Restore raw slots. i386 snapshots must have zero upper bits.
    pub fn restore(&mut self, bytes: &[u8]) -> Result<(), GuestError> {
        if bytes.len() != self.snapshot_len() {
            return Err(GuestError::cpu(
                "Guest register snapshot has the wrong architecture or length",
            ));
        }
        let count = self.slot_count();
        for (index, chunk) in bytes.chunks_exact(8).enumerate().take(count) {
            let mut word = [0u8; 8];
            word.copy_from_slice(chunk);
            let value = u64::from_le_bytes(word);
            if self.architecture == GuestArchitecture::I386 && value > 0xffff_ffff {
                return Err(GuestError::cpu(
                    "i386 snapshot has nonzero upper register bits",
                ));
            }
            self.slots[index] = value;
        }
        Ok(())
    }

    /// Raw 64-bit slot value without width checks.
    #[must_use]
    pub fn slot(&self, register: GuestRegister) -> u64 {
        self.slots[register.index()]
    }

    fn slot_count(&self) -> usize {
        match self.architecture {
            GuestArchitecture::I386 => 8,
            GuestArchitecture::X86_64 => 16,
        }
    }

    fn snapshot_len(&self) -> usize {
        self.slot_count() * 8
    }

    fn check(
        &self,
        register: GuestRegister,
        width: GuestIntegerWidth,
        high_byte: bool,
    ) -> Result<(), GuestError> {
        let index = register.index();
        if self.architecture == GuestArchitecture::I386
            && (index >= 8 || width == GuestIntegerWidth::B64)
        {
            return Err(GuestError::cpu("Register is not available in i386 mode"));
        }
        if high_byte && (width != GuestIntegerWidth::B8 || index > 3) {
            return Err(GuestError::cpu(
                "Only AH, CH, DH, and BH have high-byte register aliases",
            ));
        }
        if self.architecture == GuestArchitecture::I386
            && width == GuestIntegerWidth::B8
            && index > 3
            && index < 8
        {
            return Err(GuestError::cpu("SPL, BPL, SIL, and DIL require x86-64 mode"));
        }
        Ok(())
    }
}

/// Processor status flags. All bits are stored; instruction and ABI owners
/// apply their own writable-bit masks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProcessorFlags {
    value: u64,
}

impl ProcessorFlags {
    /// Flags with raw `value`.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self { value }
    }

    /// Raw flag word.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.value
    }

    /// Replace the raw flag word.
    pub fn set_value(&mut self, value: u64) {
        self.value = value;
    }

    /// Low 32 flag bits.
    #[must_use]
    pub const fn low_word(self) -> u32 {
        self.value as u32
    }

    /// High 32 flag bits.
    #[must_use]
    pub const fn high_word(self) -> u32 {
        (self.value >> 32) as u32
    }

    /// Restore both flag words.
    pub fn restore_words(&mut self, low: u32, high: u32) {
        self.value = (u64::from(high) << 32) | u64::from(low);
    }

    /// Replace the low flag word.
    pub fn write_low_word(&mut self, value: u32) {
        self.value = (self.value & 0xffff_ffff_0000_0000) | u64::from(value);
    }

    /// Read one flag.
    #[must_use]
    pub fn get(self, flag: GuestFlag) -> bool {
        self.low_word() & flag.mask() != 0
    }

    /// Write one flag.
    pub fn set(&mut self, flag: GuestFlag, value: bool) {
        let mask = flag.mask();
        let low = self.low_word();
        self.write_low_word(if value { low | mask } else { low & !mask });
    }
}

/// Initial environment selected by the loader or runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestProcessorInitialState {
    /// Processor architecture.
    pub architecture: GuestArchitecture,
    /// Initial instruction pointer.
    pub instruction_pointer: u64,
    /// Initial stack pointer.
    pub stack_pointer: u64,
    /// Initial flag word.
    pub flags: u64,
    /// Initial x87 control word.
    pub x87_control_word: u16,
    /// Initial MXCSR.
    pub mxcsr: u32,
    /// Initial MXCSR mask.
    pub mxcsr_mask: u32,
}

/// x87 floating-point state: eight physical 80-bit slots; TOP stays in the
/// status word and tags stay source bits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestX87State {
    /// Eight 10-byte extended-precision slots.
    pub registers: [u8; 80],
    /// Control word.
    pub control_word: u16,
    /// Status word (TOP in bits 11..14).
    pub status_word: u16,
    /// Tag word (source bits, two per slot).
    pub tag_word: u16,
    /// Last opcode.
    pub last_opcode: u16,
    /// Last instruction pointer.
    pub instruction_pointer: u64,
    /// Last data pointer.
    pub data_pointer: u64,
    /// Last instruction selector.
    pub instruction_selector: u16,
    /// Last data selector.
    pub data_selector: u16,
}

impl GuestX87State {
    /// Reset state with `control_word`.
    #[must_use]
    pub const fn new(control_word: u16) -> Self {
        Self {
            registers: [0; 80],
            control_word,
            status_word: 0,
            tag_word: 0xffff,
            last_opcode: 0,
            instruction_pointer: 0,
            data_pointer: 0,
            instruction_selector: 0,
            data_selector: 0,
        }
    }

    /// Stack top: status bits 11..14.
    #[must_use]
    pub const fn top(self) -> usize {
        ((self.status_word >> 11) & 7) as usize
    }

    /// Set the stack top, preserving the other status bits.
    pub fn set_top(&mut self, top: usize) {
        let top = (top & 7) as u16;
        self.status_word = (self.status_word & !(7 << 11)) | (top << 11);
    }
}

/// SIMD state. Raw lanes retain NaN payloads, signed zeros, integer
/// aliases, and MXCSR flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestSimdState {
    /// XMM lanes: 8 registers on i386, 16 on x86-64.
    pub xmm: Vec<u8>,
    /// MXCSR control/status.
    pub mxcsr: u32,
    /// MXCSR mask.
    pub mxcsr_mask: u32,
}

impl GuestSimdState {
    /// Zeroed SIMD state for `architecture`.
    #[must_use]
    pub fn new(architecture: GuestArchitecture, mxcsr: u32, mxcsr_mask: u32) -> Self {
        let registers = match architecture {
            GuestArchitecture::I386 => 8,
            GuestArchitecture::X86_64 => 16,
        };
        Self {
            xmm: vec![0; registers * 16],
            mxcsr,
            mxcsr_mask,
        }
    }

    /// Register count.
    #[must_use]
    pub fn register_count(&self) -> usize {
        self.xmm.len() / 16
    }
}

/// Complete guest processor state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestProcessorState {
    /// Processor architecture.
    pub architecture: GuestArchitecture,
    /// Integer registers.
    pub registers: IntegerRegisterFile,
    /// Instruction pointer.
    pub instruction_pointer: u64,
    /// Status flags.
    pub flags: ProcessorFlags,
    /// Segment descriptors: CS, DS, ES, SS, FS, GS.
    pub segments: [crate::core::contracts::GuestSegment; 6],
    /// x87 state.
    pub x87: GuestX87State,
    /// SIMD state.
    pub simd: GuestSimdState,
}

impl GuestProcessorState {
    /// Segment index of CS.
    pub const CS: usize = 0;
    /// Segment index of DS.
    pub const DS: usize = 1;
    /// Segment index of ES.
    pub const ES: usize = 2;
    /// Segment index of SS.
    pub const SS: usize = 3;
    /// Segment index of FS.
    pub const FS: usize = 4;
    /// Segment index of GS.
    pub const GS: usize = 5;

    /// Assemble the loader-selected initial environment. This does not
    /// execute instructions.
    pub fn create(initial: GuestProcessorInitialState) -> Result<Self, GuestError> {
        let width = initial.architecture.bits();
        let limit = if width == 64 {
            u128::from(u64::MAX) + 1
        } else {
            1u128 << width
        };
        if u128::from(initial.instruction_pointer) >= limit
            || u128::from(initial.stack_pointer) >= limit
        {
            return Err(GuestError::cpu(
                "Initial guest IP or stack pointer exceeds its architecture",
            ));
        }
        let mut registers = IntegerRegisterFile::new(initial.architecture);
        let stack_width = match initial.architecture {
            GuestArchitecture::I386 => GuestIntegerWidth::B32,
            GuestArchitecture::X86_64 => GuestIntegerWidth::B64,
        };
        registers.write(
            GuestRegister::Rsp,
            stack_width,
            initial.stack_pointer,
            false,
        )?;
        let segment = crate::core::contracts::GuestSegment::flat((limit - 1) as u64);
        Ok(Self {
            architecture: initial.architecture,
            registers,
            instruction_pointer: initial.instruction_pointer,
            flags: ProcessorFlags::new(initial.flags),
            segments: [segment; 6],
            x87: GuestX87State::new(initial.x87_control_word),
            simd: GuestSimdState::new(initial.architecture, initial.mxcsr, initial.mxcsr_mask),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_aliases_and_zero_extension() {
        let mut regs = IntegerRegisterFile::new(GuestArchitecture::X86_64);
        regs.write(GuestRegister::Rax, GuestIntegerWidth::B64, 0xffff_ffff_ffff_ffff, false)
            .unwrap();
        regs.write(GuestRegister::Rax, GuestIntegerWidth::B32, 0x1234_5678, false)
            .unwrap();
        assert_eq!(
            regs.read(GuestRegister::Rax, GuestIntegerWidth::B64, false)
                .unwrap(),
            0x1234_5678
        );
        regs.write(GuestRegister::Rax, GuestIntegerWidth::B8, 0xaa, true)
            .unwrap();
        assert_eq!(
            regs.read(GuestRegister::Rax, GuestIntegerWidth::B16, false)
                .unwrap(),
            0xaa78
        );
    }

    #[test]
    fn i386_rejects_upper_slots_and_snapshot_bits() {
        let mut regs = IntegerRegisterFile::new(GuestArchitecture::I386);
        assert!(regs
            .read(GuestRegister::R8, GuestIntegerWidth::B32, false)
            .is_err());
        let mut snapshot = regs.checkpoint();
        assert_eq!(snapshot.len(), 64);
        snapshot[4] = 1;
        assert!(regs.restore(&snapshot).is_err());
    }

    #[test]
    fn flags_round_trip_through_words() {
        let mut flags = ProcessorFlags::new(0);
        flags.set(GuestFlag::Zero, true);
        flags.set(GuestFlag::Carry, true);
        assert!(flags.get(GuestFlag::Zero));
        let (low, high) = (flags.low_word(), flags.high_word());
        let mut restored = ProcessorFlags::new(0);
        restored.restore_words(low, high);
        assert_eq!(restored.value(), flags.value());
    }
}
