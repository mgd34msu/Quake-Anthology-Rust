//! i386 instruction decoder: prefixes, ModRM/SIB, segments, operands.
//!
//! Donor: `src/guest/x86/decoder.ts` (`X86Decoder`). All fetching runs
//! through CS with limit checks; memory operands resolve through segment
//! bases with limit checks (`#SS` for stack segments, `#GP` otherwise).

use crate::core::contracts::{GuestAccess, GuestAddress, GuestIntegerWidth, GuestRegister};
use crate::core::memory::SparseGuestMemory;
use crate::core::registers::GuestProcessorState;
use crate::error::GuestError;

/// i386 operand width in bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum X86Width {
    /// 8-bit.
    W8,
    /// 16-bit.
    W16,
    /// 32-bit.
    W32,
}

impl X86Width {
    /// Width in bits.
    #[must_use]
    pub const fn bits(self) -> u32 {
        match self {
            Self::W8 => 8,
            Self::W16 => 16,
            Self::W32 => 32,
        }
    }

    /// Width in bytes.
    #[must_use]
    pub const fn bytes(self) -> usize {
        self.bits() as usize / 8
    }

    /// Corresponding register width.
    #[must_use]
    pub const fn register_width(self) -> GuestIntegerWidth {
        match self {
            Self::W8 => GuestIntegerWidth::B8,
            Self::W16 => GuestIntegerWidth::B16,
            Self::W32 => GuestIntegerWidth::B32,
        }
    }
}

/// Segment register name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentName {
    /// Code segment.
    Cs,
    /// Data segment.
    Ds,
    /// Extra segment.
    Es,
    /// Stack segment.
    Ss,
    /// FS segment.
    Fs,
    /// GS segment.
    Gs,
}

impl SegmentName {
    /// Segment table index.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Cs => GuestProcessorState::CS,
            Self::Ds => GuestProcessorState::DS,
            Self::Es => GuestProcessorState::ES,
            Self::Ss => GuestProcessorState::SS,
            Self::Fs => GuestProcessorState::FS,
            Self::Gs => GuestProcessorState::GS,
        }
    }

    /// Segment label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Cs => "cs",
            Self::Ds => "ds",
            Self::Es => "es",
            Self::Ss => "ss",
            Self::Fs => "fs",
            Self::Gs => "gs",
        }
    }
}

/// Decoded operand: register or segmented memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum X86Operand {
    /// Register with its ModRM index and high-byte alias flag.
    Register {
        /// Register.
        register: GuestRegister,
        /// AH/CH/DH/BH alias.
        high_byte: bool,
        /// ModRM register index.
        index: usize,
    },
    /// Segmented memory with a wrapped offset.
    Memory {
        /// Wrapped effective offset.
        offset: u64,
        /// Segment.
        segment: SegmentName,
        /// ESP-based addressing (affects POP adjustment).
        stack_pointer_base: bool,
    },
}

/// Decoded ModRM byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct X86ModRM {
    /// Raw ModRM byte.
    pub byte: u8,
    /// Register/opcode field.
    pub group: usize,
    /// Register field as an operand.
    pub register: X86Operand,
    /// R/M field as an operand.
    pub operand: X86Operand,
}

/// REP prefix state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RepeatPrefix {
    /// No REP prefix.
    #[default]
    None,
    /// `F2` (`REPNE`).
    F2,
    /// `F3` (`REP`/`REPE`).
    F3,
}

/// i386 step failure: memory fault, processor fault, or unsupported encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum X86Error {
    /// Guest memory fault.
    Memory {
        /// Requested access.
        access: GuestAccess,
        /// Faulting offset.
        address: u64,
        /// Requested length.
        byte_length: usize,
        /// Human-readable detail.
        detail: String,
    },
    /// Processor fault with vector and optional error code.
    Fault {
        /// Exception vector.
        vector: u32,
        /// Error code, if pushed.
        error_code: Option<u64>,
        /// Human-readable detail.
        detail: String,
    },
    /// Undecodable or unimplemented encoding.
    Unsupported(String),
}

impl X86Error {
    /// Processor fault without an error code.
    #[must_use]
    pub fn fault(vector: u32, detail: impl Into<String>) -> Self {
        Self::Fault {
            vector,
            error_code: None,
            detail: detail.into(),
        }
    }

    /// Processor fault with an error code.
    #[must_use]
    pub fn fault_code(vector: u32, detail: impl Into<String>, error_code: u64) -> Self {
        Self::Fault {
            vector,
            error_code: Some(error_code),
            detail: detail.into(),
        }
    }

    /// Unsupported encoding.
    #[must_use]
    pub fn unsupported(detail: impl Into<String>) -> Self {
        Self::Unsupported(detail.into())
    }
}

impl From<GuestError> for X86Error {
    fn from(error: GuestError) -> Self {
        match error {
            GuestError::MemoryFault {
                reason: _,
                address,
                length,
                access,
                detail,
            } => {
                let access = match access {
                    "write" => GuestAccess::Write,
                    "execute" => GuestAccess::Execute,
                    _ => GuestAccess::Read,
                };
                Self::Memory {
                    access,
                    address,
                    byte_length: length,
                    detail,
                }
            }
            _ => Self::Unsupported(error.to_string()),
        }
    }
}

/// Non-null guest address in `memory`.
pub fn guest_address(
    memory: &mut SparseGuestMemory,
    raw: u64,
    access: GuestAccess,
    length: usize,
) -> Result<GuestAddress, X86Error> {
    memory.pointer(raw)?.ok_or_else(|| X86Error::Memory {
        access,
        address: raw,
        byte_length: length,
        detail: "null guest address".to_string(),
    })
}

/// Decode a ModRM register index at `width`.
pub fn register_operand(index: usize, width: X86Width) -> Result<X86Operand, X86Error> {
    let high_byte = width == X86Width::W8 && index >= 4;
    let actual = if high_byte { index - 4 } else { index };
    let register = match actual {
        0 => GuestRegister::Rax,
        1 => GuestRegister::Rcx,
        2 => GuestRegister::Rdx,
        3 => GuestRegister::Rbx,
        4 => GuestRegister::Rsp,
        5 => GuestRegister::Rbp,
        6 => GuestRegister::Rsi,
        7 => GuestRegister::Rdi,
        _ => return Err(X86Error::unsupported(format!("Invalid i386 register index {index}"))),
    };
    Ok(X86Operand::Register {
        register,
        high_byte,
        index,
    })
}

/// Streaming i386 decoder over live guest state and memory.
pub struct X86Decoder<'a> {
    /// Borrowed processor state.
    pub state: &'a mut GuestProcessorState,
    /// Borrowed guest memory.
    pub memory: &'a mut SparseGuestMemory,
    /// Instruction start IP.
    pub start: u64,
    /// Consumed bytes.
    pub bytes: Vec<u8>,
    /// Decode cursor (next IP unless redirected).
    pub cursor: u64,
    /// Operand width in bits (16 or 32).
    pub operand_bits: u32,
    /// Address width in bits (16 or 32).
    pub address_bits: u32,
    /// Segment override, if any.
    pub segment: Option<SegmentName>,
    /// REP prefix, if any.
    pub repeat: RepeatPrefix,
    /// LOCK prefix present.
    pub lock: bool,
    /// Primary opcode byte.
    pub opcode: u8,
}

impl<'a> X86Decoder<'a> {
    /// Decode prefixes and the opcode at the current IP.
    pub fn new(state: &'a mut GuestProcessorState, memory: &'a mut SparseGuestMemory) -> Result<Self, X86Error> {
        let start = state.instruction_pointer;
        let mut decoder = Self {
            state,
            memory,
            start,
            bytes: Vec::new(),
            cursor: start,
            operand_bits: 32,
            address_bits: 32,
            segment: None,
            repeat: RepeatPrefix::None,
            lock: false,
            opcode: 0,
        };
        loop {
            match decoder.byte()? {
                0x66 => decoder.operand_bits = 16,
                0x67 => decoder.address_bits = 16,
                0xf0 => decoder.lock = true,
                0xf2 => decoder.repeat = RepeatPrefix::F2,
                0xf3 => decoder.repeat = RepeatPrefix::F3,
                0x26 => decoder.segment = Some(SegmentName::Es),
                0x2e => decoder.segment = Some(SegmentName::Cs),
                0x36 => decoder.segment = Some(SegmentName::Ss),
                0x3e => decoder.segment = Some(SegmentName::Ds),
                0x64 => decoder.segment = Some(SegmentName::Fs),
                0x65 => decoder.segment = Some(SegmentName::Gs),
                opcode => {
                    decoder.opcode = opcode;
                    return Ok(decoder);
                }
            }
        }
    }

    /// Fetch one byte through CS with limit checks.
    pub fn byte(&mut self) -> Result<u8, X86Error> {
        if self.bytes.len() >= 15 {
            return Err(X86Error::fault_code(
                13,
                "Instruction exceeds the 15-byte architectural limit",
                0,
            ));
        }
        if self.cursor > self.state.segments[GuestProcessorState::CS].limit {
            return Err(X86Error::fault_code(13, "Instruction exceeds CS limit", 0));
        }
        let address = (self.state.segments[GuestProcessorState::CS]
            .base
            .wrapping_add(self.cursor)) as u32;
        let byte = self.memory.fetch_byte(u64::from(address))?;
        self.bytes.push(byte);
        self.cursor = self.cursor.wrapping_add(1) as u32 as u64;
        Ok(byte)
    }

    /// Fetch an unsigned immediate of `width`.
    pub fn immediate(&mut self, width: X86Width) -> Result<u64, X86Error> {
        let mut value = 0u64;
        for offset in (0..width.bits()).step_by(8) {
            value |= u64::from(self.byte()?) << offset;
        }
        Ok(value)
    }

    /// Fetch a sign-extended immediate of `width`.
    pub fn signed(&mut self, width: X86Width) -> Result<i64, X86Error> {
        let value = self.immediate(width)?;
        Ok(match width {
            X86Width::W8 => i64::from(value as u8 as i8),
            X86Width::W16 => i64::from(value as u16 as i16),
            X86Width::W32 => i64::from(value as u32 as i32),
        })
    }

    /// Read a full register by ModRM index.
    pub fn register_value(&mut self, index: usize, width: X86Width) -> Result<u64, X86Error> {
        let operand = register_operand(index, width)?;
        let X86Operand::Register {
            register, high_byte, ..
        } = operand
        else {
            return Err(X86Error::unsupported("Expected decoded register"));
        };
        Ok(self.state.registers.read(register, width.register_width(), high_byte)?)
    }

    /// Decode a ModRM byte (plus SIB/displacement) at `width`.
    pub fn modrm(&mut self, width: X86Width) -> Result<X86ModRM, X86Error> {
        let byte = self.byte();
        let byte = byte?;
        let mode = byte >> 6;
        let group = ((byte >> 3) & 7) as usize;
        let rm = byte & 7;
        let register = register_operand(group, width)?;
        if mode == 3 {
            return Ok(X86ModRM {
                byte,
                group,
                register,
                operand: register_operand(rm as usize, width)?,
            });
        }
        let mut offset: i64 = 0;
        let mut stack = false;
        let mut stack_pointer_base = false;
        if self.address_bits == 16 {
            let bx = self.register_value(3, X86Width::W16)? as i64;
            let bp = self.register_value(5, X86Width::W16)? as i64;
            let si = self.register_value(6, X86Width::W16)? as i64;
            let di = self.register_value(7, X86Width::W16)? as i64;
            match rm {
                0 => offset = bx + si,
                1 => offset = bx + di,
                2 => {
                    offset = bp + si;
                    stack = true;
                }
                3 => {
                    offset = bp + di;
                    stack = true;
                }
                4 => offset = si,
                5 => offset = di,
                6 => {
                    if mode == 0 {
                        offset = self.immediate(X86Width::W16)? as i64;
                    } else {
                        offset = bp;
                        stack = true;
                    }
                }
                _ => offset = bx,
            }
            if mode == 1 {
                offset += self.signed(X86Width::W8)?;
            }
            if mode == 2 {
                offset += self.signed(X86Width::W16)?;
            }
        } else {
            match rm {
                4 => {
                    let sib = self.byte()?;
                    let scale = sib >> 6;
                    let index = (sib >> 3) & 7;
                    let base = sib & 7;
                    if index != 4 {
                        offset += (self.register_value(index as usize, X86Width::W32)? as i64) << scale;
                    }
                    if base == 5 && mode == 0 {
                        offset += self.immediate(X86Width::W32)? as i64;
                    } else {
                        offset += self.register_value(base as usize, X86Width::W32)? as i64;
                        stack = base == 4 || base == 5;
                        stack_pointer_base = base == 4;
                    }
                }
                5 if mode == 0 => offset = self.immediate(X86Width::W32)? as i64,
                _ => {
                    offset = self.register_value(rm as usize, X86Width::W32)? as i64;
                    stack = rm == 5;
                }
            }
            if mode == 1 {
                offset += self.signed(X86Width::W8)?;
            }
            if mode == 2 {
                offset += self.signed(X86Width::W32)?;
            }
        }
        let wrapped = if self.address_bits == 16 {
            (offset as u64) & 0xffff
        } else {
            (offset as u64) & 0xffff_ffff
        };
        Ok(X86ModRM {
            byte,
            group,
            register,
            operand: X86Operand::Memory {
                offset: wrapped,
                segment: self
                    .segment
                    .unwrap_or(if stack { SegmentName::Ss } else { SegmentName::Ds }),
                stack_pointer_base,
            },
        })
    }

    /// Resolve a memory operand through its segment with limit checks.
    pub fn address(
        &mut self,
        operand: X86Operand,
        width_bytes: usize,
        access: GuestAccess,
    ) -> Result<GuestAddress, X86Error> {
        let X86Operand::Memory { offset, segment, .. } = operand else {
            return Err(X86Error::unsupported("Expected memory operand"));
        };
        let descriptor = self.state.segments[segment.index()];
        if offset + width_bytes as u64 - 1 > descriptor.limit {
            return Err(X86Error::fault_code(
                if segment == SegmentName::Ss { 12 } else { 13 },
                format!("{} segment limit exceeded", segment.label()),
                0,
            ));
        }
        guest_address(
            self.memory,
            descriptor.base.wrapping_add(offset) as u32 as u64,
            access,
            width_bytes,
        )
    }

    /// Read an operand at `width`.
    pub fn read(&mut self, operand: X86Operand, width: X86Width) -> Result<u64, X86Error> {
        match operand {
            X86Operand::Register {
                register, high_byte, ..
            } => Ok(self.state.registers.read(register, width.register_width(), high_byte)?),
            X86Operand::Memory { .. } => {
                let address = self.address(operand, width.bytes(), GuestAccess::Read)?;
                Ok(match width {
                    X86Width::W8 => u64::from(self.memory.read_u8(address)?),
                    X86Width::W16 => u64::from(self.memory.read_u16(address)?),
                    X86Width::W32 => u64::from(self.memory.read_u32(address)?),
                })
            }
        }
    }

    /// Pre-check a memory write before flags are updated.
    pub fn check_write(&mut self, operand: X86Operand, width: X86Width) -> Result<(), X86Error> {
        if matches!(operand, X86Operand::Memory { .. }) {
            let address = self.address(operand, width.bytes(), GuestAccess::Write)?;
            self.memory.check(address, width.bytes(), GuestAccess::Write)?;
        }
        Ok(())
    }

    /// Write an operand at `width`.
    pub fn write(&mut self, operand: X86Operand, width: X86Width, value: u64) -> Result<(), X86Error> {
        match operand {
            X86Operand::Register {
                register, high_byte, ..
            } => Ok(self
                .state
                .registers
                .write(register, width.register_width(), value, high_byte)?),
            X86Operand::Memory { .. } => {
                let address = self.address(operand, width.bytes(), GuestAccess::Write)?;
                match width {
                    X86Width::W8 => self.memory.write_u8(address, value as u8)?,
                    X86Width::W16 => self.memory.write_u16(address, value as u16)?,
                    X86Width::W32 => self.memory.write_u32(address, value as u32)?,
                }
                Ok(())
            }
        }
    }
}
