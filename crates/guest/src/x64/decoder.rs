//! x86-64 instruction decoder with replayable decode records.
//!
//! Donor: `src/guest/x64/decoder.ts` (`X64DecodeCursor`). Follows AMD APM
//! volume 3 sections 1.2-1.7; memory addresses wait until all immediates
//! are decoded. Decoded bytes are retained with an executable-bytes guard
//! so cached plans revalidate against live memory.

use crate::core::contracts::{GuestAccess, GuestAddress, GuestIntegerWidth, GuestRegister};
use crate::core::memory::{ExecutableRetention, FetchCursor, SparseGuestMemory};
use crate::core::registers::GuestProcessorState;
use crate::x64::plan::X64SemanticPlan;
use crate::x86::decoder::X86Error;

/// Canonical 48-bit low half (`0x0000_0000_0000_0000..=0x0000_7fff_ffff_ffff`).
pub const CANONICAL_LOW_MAX: u64 = 0x0000_7fff_ffff_ffff;
/// Canonical 48-bit high half base (`0xffff_8000_0000_0000..=u64::MAX`).
pub const CANONICAL_HIGH_MIN: u64 = 0xffff_8000_0000_0000;

/// Canonicalize a 64-bit virtual address, faulting `#GP` on the hole.
pub fn canonical_address(value: u64) -> Result<u64, X86Error> {
    if value <= CANONICAL_LOW_MAX || value >= CANONICAL_HIGH_MIN {
        Ok(value)
    } else {
        Err(X86Error::fault(
            13,
            format!("Noncanonical 48-bit virtual address 0x{value:x}"),
        ))
    }
}

/// Non-null transient operand address. The CPU owns this transient operand;
/// checked memory access retains mapping admission.
pub fn guest_address(space: u64, raw: u64, access: GuestAccess) -> Result<GuestAddress, X86Error> {
    let value = canonical_address(raw)?;
    if value == 0 {
        return Err(X86Error::Memory {
            access,
            address: value,
            byte_length: 1,
            detail: "null guest address".to_string(),
        });
    }
    Ok(GuestAddress::new(space, value))
}

/// Checked memory load at `width`.
pub fn read_memory(
    memory: &mut SparseGuestMemory,
    address: GuestAddress,
    width: GuestIntegerWidth,
) -> Result<u64, X86Error> {
    Ok(match width {
        GuestIntegerWidth::B8 => u64::from(memory.read_u8(address)?),
        GuestIntegerWidth::B16 => u64::from(memory.read_u16(address)?),
        GuestIntegerWidth::B32 => u64::from(memory.read_u32(address)?),
        GuestIntegerWidth::B64 => memory.read_u64(address)?,
    })
}

/// Checked memory store at `width`.
pub fn write_memory(
    memory: &mut SparseGuestMemory,
    address: GuestAddress,
    width: GuestIntegerWidth,
    value: u64,
) -> Result<(), X86Error> {
    match width {
        GuestIntegerWidth::B8 => memory.write_u8(address, value as u8)?,
        GuestIntegerWidth::B16 => memory.write_u16(address, value as u16)?,
        GuestIntegerWidth::B32 => memory.write_u32(address, value as u32)?,
        GuestIntegerWidth::B64 => memory.write_u64(address, value)?,
    }
    Ok(())
}

/// x86-64 register operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct X64RegisterOperand {
    /// Register.
    pub register: GuestRegister,
    /// Operand width.
    pub width: GuestIntegerWidth,
    /// AH/CH/DH/BH alias (only without REX).
    pub high_byte: bool,
}

/// x86-64 memory operand (unresolved SIB form).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct X64MemoryOperand {
    /// Operand width.
    pub width: GuestIntegerWidth,
    /// Base register, if any.
    pub base: Option<GuestRegister>,
    /// Index register, if any.
    pub index: Option<GuestRegister>,
    /// Index scale (1, 2, 4, or 8).
    pub scale: u64,
    /// Signed displacement.
    pub displacement: i64,
    /// RIP-relative addressing.
    pub rip_relative: bool,
    /// Address width in bits.
    pub address_bits: u32,
    /// FS/GS segment override, if any.
    pub segment: Option<X64Segment>,
}

/// Segment override meaningful in 64-bit mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum X64Segment {
    /// FS base.
    Fs,
    /// GS base.
    Gs,
}

impl X64Segment {
    /// Segment table index.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Fs => GuestProcessorState::FS,
            Self::Gs => GuestProcessorState::GS,
        }
    }
}

/// x86-64 operand: register or unresolved memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum X64Operand {
    /// Register operand.
    Register(X64RegisterOperand),
    /// Memory operand.
    Memory(X64MemoryOperand),
}

impl X64Operand {
    /// Operand width.
    #[must_use]
    pub const fn width(self) -> GuestIntegerWidth {
        match self {
            Self::Register(operand) => operand.width,
            Self::Memory(operand) => operand.width,
        }
    }
}

/// Decoded ModRM byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct X64ModRM {
    /// Raw ModRM byte.
    pub byte: u8,
    /// Register/opcode field (before REX.R).
    pub extension: usize,
    /// Full register index (with REX.R).
    pub register_index: usize,
    /// Full R/M index (with REX.B).
    pub rm_index: usize,
    /// Register field as an operand.
    pub reg: X64RegisterOperand,
    /// R/M field as an operand.
    pub rm: X64Operand,
}

/// Recorded ModRM decode for replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodedOperand {
    /// Decode width.
    pub width: GuestIntegerWidth,
    /// Cursor end position.
    pub end: usize,
    /// Decoded value.
    pub value: X64ModRM,
}

/// Recorded immediate decode for replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodedImmediate {
    /// Immediate length in bytes.
    pub length: usize,
    /// Decoded value.
    pub value: u64,
}

/// Retained decode records for one instruction.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DecodeRecords {
    /// Consumed bytes.
    pub bytes: Vec<u8>,
    /// ModRM records by start position.
    pub operands: Vec<Option<DecodedOperand>>,
    /// Immediate records by start position.
    pub immediates: Vec<Option<DecodedImmediate>>,
}

/// Cached decoded instruction with its liveness guard.
#[derive(Debug, Clone)]
pub struct X64DecodedInstruction {
    /// Consumed bytes.
    pub bytes: Vec<u8>,
    /// Prefix length in bytes.
    pub prefix_length: usize,
    /// Primary opcode.
    pub opcode: u8,
    /// REX prefix, if any.
    pub rex: Option<u8>,
    /// Operand-size override.
    pub operand_override: bool,
    /// Address-size override.
    pub address_override: bool,
    /// LOCK prefix.
    pub lock: bool,
    /// REP prefix.
    pub repeat: X64Repeat,
    /// Segment override.
    pub segment: Option<X64Segment>,
    /// ModRM records by start position.
    pub operands: Vec<Option<DecodedOperand>>,
    /// Immediate records by start position.
    pub immediates: Vec<Option<DecodedImmediate>>,
    /// Liveness guard against live memory.
    pub retention: ExecutableRetention,
}

/// REP prefix state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum X64Repeat {
    /// No REP prefix.
    #[default]
    None,
    /// `F2` (`REPNE`).
    F2,
    /// `F3` (`REP`/`REPE`).
    F3,
}

/// Effective offset of a memory operand (before segment base).
pub fn effective_operand_offset(
    state: &GuestProcessorState,
    operand: &X64MemoryOperand,
    next_ip: u64,
) -> Result<u64, X86Error> {
    let address_width = if operand.address_bits == 32 {
        GuestIntegerWidth::B32
    } else {
        GuestIntegerWidth::B64
    };
    let base = if operand.rip_relative {
        next_ip as i128
    } else if let Some(base) = operand.base {
        state.registers.read(base, address_width, false)? as i128
    } else {
        0
    };
    let index = if let Some(index) = operand.index {
        state.registers.read(index, address_width, false)? as i128 * operand.scale as i128
    } else {
        0
    };
    let offset = base + index + operand.displacement as i128;
    Ok(if operand.address_bits == 32 {
        offset as u64 & 0xffff_ffff
    } else {
        offset as u64
    })
}

/// Resolve a memory operand through its segment base.
pub fn operand_address(
    memory: &SparseGuestMemory,
    state: &GuestProcessorState,
    operand: &X64MemoryOperand,
    next_ip: u64,
    access: GuestAccess,
) -> Result<GuestAddress, X86Error> {
    let segment_base = operand
        .segment
        .map_or(0, |segment| state.segments[segment.index()].base);
    guest_address(
        memory.address_space(),
        effective_operand_offset(state, operand, next_ip)?.wrapping_add(segment_base),
        access,
    )
}

/// Read an operand (memory through `next_ip` for RIP-relative).
pub fn read_operand(
    memory: &mut SparseGuestMemory,
    state: &GuestProcessorState,
    operand: &X64Operand,
    next_ip: u64,
) -> Result<u64, X86Error> {
    match operand {
        X64Operand::Register(operand) => {
            Ok(state.registers.read(operand.register, operand.width, operand.high_byte)?)
        }
        X64Operand::Memory(operand) => {
            let address = operand_address(memory, state, operand, next_ip, GuestAccess::Read)?;
            read_memory(memory, address, operand.width)
        }
    }
}

/// Write an operand (memory through `next_ip` for RIP-relative).
pub fn write_operand(
    memory: &mut SparseGuestMemory,
    state: &mut GuestProcessorState,
    operand: &X64Operand,
    next_ip: u64,
    value: u64,
) -> Result<(), X86Error> {
    match operand {
        X64Operand::Register(operand) => {
            Ok(state.registers.write(operand.register, operand.width, value, operand.high_byte)?)
        }
        X64Operand::Memory(operand) => {
            let address = operand_address(memory, state, operand, next_ip, GuestAccess::Write)?;
            write_memory(memory, address, operand.width, value)
        }
    }
}

/// Pre-check a memory write before flags are updated.
pub fn writable_operand(
    memory: &mut SparseGuestMemory,
    state: &GuestProcessorState,
    operand: &X64Operand,
    next_ip: u64,
) -> Result<(), X86Error> {
    if let X64Operand::Memory(operand) = operand {
        let address = operand_address(memory, state, operand, next_ip, GuestAccess::Write)?;
        memory.check(address, operand.width.bytes(), GuestAccess::Write)?;
    }
    Ok(())
}

/// Streaming x86-64 decoder with optional replay of retained records.
pub struct X64DecodeCursor<'a> {
    /// Borrowed guest memory.
    pub memory: &'a mut SparseGuestMemory,
    /// Borrowed processor state.
    pub state: &'a mut GuestProcessorState,
    start: u64,
    decoded: Option<X64DecodedInstruction>,
    records: Option<DecodeRecords>,
    fetch: Option<FetchCursor>,
    prefix_length: usize,
    position: usize,
    opcode: u8,
    /// Built semantic plan (set by the CPU's planner).
    pub plan: Option<X64SemanticPlan>,
    /// REX prefix, if any.
    pub rex: Option<u8>,
    /// Operand-size override.
    pub operand_override: bool,
    /// Address-size override.
    pub address_override: bool,
    /// LOCK prefix.
    pub lock: bool,
    /// REP prefix.
    pub repeat: X64Repeat,
    /// Segment override.
    pub segment: Option<X64Segment>,
}

impl<'a> X64DecodeCursor<'a> {
    /// Decoder at the current IP, optionally replaying `decoded`.
    pub fn new(
        memory: &'a mut SparseGuestMemory,
        state: &'a mut GuestProcessorState,
        decoded: Option<X64DecodedInstruction>,
    ) -> Result<Self, X86Error> {
        let mut cursor = Self {
            memory,
            state,
            start: 0,
            decoded,
            records: None,
            fetch: None,
            prefix_length: 0,
            position: 0,
            opcode: 0,
            plan: None,
            rex: None,
            operand_override: false,
            address_override: false,
            lock: false,
            repeat: X64Repeat::None,
            segment: None,
        };
        cursor.reset()?;
        Ok(cursor)
    }

    fn reset(&mut self) -> Result<(), X86Error> {
        self.start = self.state.instruction_pointer;
        self.plan = None;
        if let Some(decoded) = self.decoded.take() {
            self.position = decoded.prefix_length;
            self.prefix_length = decoded.prefix_length;
            self.opcode = decoded.opcode;
            self.rex = decoded.rex;
            self.operand_override = decoded.operand_override;
            self.address_override = decoded.address_override;
            self.lock = decoded.lock;
            self.repeat = decoded.repeat;
            self.segment = decoded.segment;
            self.decoded = Some(decoded);
            self.records = None;
            self.fetch = None;
            return Ok(());
        }
        self.position = 0;
        self.rex = None;
        self.operand_override = false;
        self.address_override = false;
        self.lock = false;
        self.repeat = X64Repeat::None;
        self.segment = None;
        self.records = Some(DecodeRecords::default());
        // The complete architectural instruction window stays canonical and
        // cannot wrap. Boundary instructions retain the per-byte path below.
        self.fetch = if (self.start > 0 && self.start <= 0x0000_7fff_ffff_fff1)
            || (self.start >= CANONICAL_HIGH_MIN && self.start <= 0xffff_ffff_ffff_fff1)
        {
            Some(FetchCursor::new(self.start))
        } else {
            None
        };
        loop {
            let byte = self.read_byte()?;
            if (0x40..=0x4f).contains(&byte) {
                self.rex = Some(byte);
                continue;
            }
            if byte == 0x66 {
                self.operand_override = true;
            } else if byte == 0x67 {
                self.address_override = true;
            } else if byte == 0xf0 {
                self.lock = true;
            } else if byte == 0xf2 {
                self.repeat = X64Repeat::F2;
            } else if byte == 0xf3 {
                self.repeat = X64Repeat::F3;
            } else if byte == 0x64 {
                self.segment = Some(X64Segment::Fs);
            } else if byte == 0x65 {
                self.segment = Some(X64Segment::Gs);
            } else if matches!(byte, 0x2e | 0x36 | 0x3e | 0x26) {
                self.segment = None;
            } else {
                self.opcode = byte;
                break;
            }
            self.rex = None;
        }
        self.prefix_length = self.position;
        Ok(())
    }

    /// Instruction start IP.
    #[must_use]
    pub const fn start(&self) -> u64 {
        self.start
    }

    /// Primary opcode.
    #[must_use]
    pub const fn opcode(&self) -> u8 {
        self.opcode
    }

    /// Consumed bytes (replay window or live records).
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        if let Some(decoded) = &self.decoded {
            &decoded.bytes[..self.position.min(decoded.bytes.len())]
        } else if let Some(records) = &self.records {
            &records.bytes
        } else {
            &[]
        }
    }

    /// Retain the decoded bytes for caching.
    pub fn cache(&mut self) -> Option<X64DecodedInstruction> {
        if let Some(decoded) = self.decoded.clone() {
            return Some(decoded);
        }
        let records = self.records.clone()?;
        let retention = self.memory.retain_executable_bytes(self.start, &records.bytes)?;
        Some(X64DecodedInstruction {
            bytes: records.bytes,
            prefix_length: self.prefix_length,
            opcode: self.opcode,
            rex: self.rex,
            operand_override: self.operand_override,
            address_override: self.address_override,
            lock: self.lock,
            repeat: self.repeat,
            segment: self.segment,
            operands: records.operands,
            immediates: records.immediates,
            retention,
        })
    }

    /// Next IP after the consumed bytes.
    #[must_use]
    pub fn next_ip(&self) -> u64 {
        self.start.wrapping_add(self.position as u64)
    }

    /// Operand width in bits.
    #[must_use]
    pub fn width(&self) -> GuestIntegerWidth {
        if self.rex.is_some_and(|rex| rex & 8 != 0) {
            GuestIntegerWidth::B64
        } else if self.operand_override {
            GuestIntegerWidth::B16
        } else {
            GuestIntegerWidth::B32
        }
    }

    /// Stack width in bits.
    #[must_use]
    pub fn stack_width(&self) -> GuestIntegerWidth {
        if self.operand_override {
            GuestIntegerWidth::B16
        } else {
            GuestIntegerWidth::B64
        }
    }

    /// Address width in bits.
    #[must_use]
    pub fn address_bits(&self) -> u32 {
        if self.address_override {
            32
        } else {
            64
        }
    }

    /// REX.B extension bit.
    #[must_use]
    pub fn rex_b(&self) -> usize {
        if self.rex.is_some_and(|rex| rex & 1 != 0) {
            8
        } else {
            0
        }
    }

    /// REX.X extension bit.
    #[must_use]
    pub fn rex_x(&self) -> usize {
        if self.rex.is_some_and(|rex| rex & 2 != 0) {
            8
        } else {
            0
        }
    }

    /// REX.R extension bit.
    #[must_use]
    pub fn rex_r(&self) -> usize {
        if self.rex.is_some_and(|rex| rex & 4 != 0) {
            8
        } else {
            0
        }
    }

    /// Numeric mandatory prefix.
    #[must_use]
    pub fn numeric_prefix(&self) -> crate::floating_point::contracts::NumericPrefix {
        use crate::floating_point::contracts::NumericPrefix;
        match self.repeat {
            X64Repeat::F2 => NumericPrefix::XF2,
            X64Repeat::F3 => NumericPrefix::XF3,
            X64Repeat::None => {
                if self.operand_override {
                    NumericPrefix::X66
                } else {
                    NumericPrefix::None
                }
            }
        }
    }

    /// Fetch one byte, replaying retained records when present.
    pub fn read_byte(&mut self) -> Result<u8, X86Error> {
        if self.position >= 15 {
            return Err(X86Error::fault(13, "Instruction exceeds 15 bytes"));
        }
        if let Some(decoded) = &self.decoded {
            let byte = decoded.bytes.get(self.position).copied().ok_or_else(|| {
                X86Error::unsupported("Decoded instruction byte is missing")
            })?;
            self.position += 1;
            return Ok(byte);
        }
        let byte = match &mut self.fetch {
            Some(cursor) => self.memory.fetch_sequence_byte(cursor)?,
            None => self.memory.fetch_byte(canonical_address(self.next_ip())?)?,
        };
        if let Some(records) = self.records.as_mut() {
            records.bytes.push(byte);
        }
        self.position += 1;
        Ok(byte)
    }

    /// Fetch an unsigned immediate of `byte_length` bytes.
    pub fn read_unsigned(&mut self, byte_length: usize) -> Result<u64, X86Error> {
        let start = self.position;
        if let Some(decoded) = &self.decoded {
            if let Some(Some(saved)) = decoded.immediates.get(start) {
                if saved.length == byte_length {
                    self.position += byte_length;
                    return Ok(saved.value);
                }
            }
        }
        let value = self.unsigned(byte_length)?;
        if let Some(records) = self.records.as_mut() {
            if records.immediates.len() <= start {
                records.immediates.resize(start + 1, None);
            }
            records.immediates[start] = Some(DecodedImmediate {
                length: byte_length,
                value,
            });
        }
        Ok(value)
    }

    fn unsigned(&mut self, byte_length: usize) -> Result<u64, X86Error> {
        if byte_length == 1 {
            return Ok(u64::from(self.read_byte()?));
        }
        if byte_length == 2 {
            let low = u64::from(self.read_byte()?);
            let high = u64::from(self.read_byte()?);
            return Ok(low | high << 8);
        }
        if byte_length == 4 {
            let mut value = 0u64;
            for shift in [0, 8, 16, 24] {
                value |= u64::from(self.read_byte()?) << shift;
            }
            return Ok(value);
        }
        let mut value = 0u64;
        for index in 0..byte_length {
            value |= u64::from(self.read_byte()?) << (index * 8);
        }
        Ok(value)
    }

    /// Fetch a sign-extended immediate of `byte_length` bytes.
    pub fn read_signed(&mut self, byte_length: usize) -> Result<i64, X86Error> {
        let value = self.read_unsigned(byte_length)?;
        Ok(match byte_length {
            1 => i64::from(value as u8 as i8),
            2 => i64::from(value as u16 as i16),
            4 => i64::from(value as u32 as i32),
            _ => value as i64,
        })
    }

    /// Fetch an ALU immediate: 64-bit operands take sign-extended 32-bit.
    pub fn immediate(&mut self, width: GuestIntegerWidth) -> Result<u64, X86Error> {
        if width == GuestIntegerWidth::B64 {
            Ok(self.read_signed(4)? as u64)
        } else {
            self.read_unsigned(width.bytes())
        }
    }

    /// Decode a register operand at full `index` with REX high-byte rules.
    pub fn register(
        &self,
        index: usize,
        width: GuestIntegerWidth,
    ) -> Result<X64RegisterOperand, X86Error> {
        if width == GuestIntegerWidth::B8 && self.rex.is_none() && (4..8).contains(&index) {
            return Ok(X64RegisterOperand {
                register: GuestRegister::decode((index - 4) as u8, false),
                width,
                high_byte: true,
            });
        }
        if index >= 16 {
            return Err(X86Error::fault(6, format!("Invalid register index {index}")));
        }
        Ok(X64RegisterOperand {
            register: GuestRegister::decode((index & 7) as u8, index >= 8),
            width,
            high_byte: false,
        })
    }

    /// Decode a ModRM byte (plus SIB/displacement) at `width`.
    pub fn decode_modrm(&mut self, width: GuestIntegerWidth) -> Result<X64ModRM, X86Error> {
        let start = self.position;
        if let Some(decoded) = &self.decoded {
            if let Some(Some(saved)) = decoded.operands.get(start) {
                if saved.width == width {
                    self.position = saved.end;
                    return Ok(saved.value);
                }
            }
        }
        let value = self.modrm(width)?;
        if let Some(records) = self.records.as_mut() {
            if records.operands.len() <= start {
                records.operands.resize(start + 1, None);
            }
            records.operands[start] = Some(DecodedOperand {
                width,
                end: self.position,
                value,
            });
        }
        Ok(value)
    }

    fn modrm(&mut self, width: GuestIntegerWidth) -> Result<X64ModRM, X86Error> {
        let byte = self.read_byte()?;
        let mode = byte >> 6;
        let extension = ((byte >> 3) & 7) as usize;
        let low_rm = byte & 7;
        let register_index = extension + self.rex_r();
        let rm_index = low_rm as usize + self.rex_b();
        let reg = self.register(register_index, width)?;
        if mode == 3 {
            return Ok(X64ModRM {
                byte,
                extension,
                register_index,
                rm_index,
                reg,
                rm: X64Operand::Register(self.register(rm_index, width)?),
            });
        }
        let mut base = None;
        let mut index = None;
        let mut scale = 1u64;
        let mut displacement = 0i64;
        let mut rip_relative = false;
        if low_rm == 4 {
            let sib = self.read_byte()?;
            let low_base = sib & 7;
            let low_index = (sib >> 3) & 7;
            scale = 1u64 << (sib >> 6);
            if low_index != 4 || self.rex_x() != 0 {
                index = Some(GuestRegister::decode(low_index, self.rex_x() != 0));
            }
            if mode == 0 && low_base == 5 {
                displacement = self.read_signed(4)?;
            } else {
                base = Some(GuestRegister::decode(low_base, self.rex_b() != 0));
            }
        } else if mode == 0 && low_rm == 5 {
            rip_relative = true;
            displacement = self.read_signed(4)?;
        } else {
            base = Some(GuestRegister::decode(low_rm, self.rex_b() != 0));
        }
        if mode == 1 {
            displacement = self.read_signed(1)?;
        } else if mode == 2 {
            displacement = self.read_signed(4)?;
        }
        Ok(X64ModRM {
            byte,
            extension,
            register_index,
            rm_index,
            reg,
            rm: X64Operand::Memory(X64MemoryOperand {
                width,
                base,
                index,
                scale,
                displacement,
                rip_relative,
                address_bits: self.address_bits(),
                segment: self.segment,
            }),
        })
    }

    /// Effective offset of a memory operand.
    pub fn effective_offset(&self, operand: &X64MemoryOperand) -> Result<u64, X86Error> {
        effective_operand_offset(
            self.state,
            operand,
            if operand.rip_relative {
                self.next_ip()
            } else {
                0
            },
        )
    }

    /// Resolve a memory operand address.
    pub fn address(
        &mut self,
        operand: &X64MemoryOperand,
        access: GuestAccess,
    ) -> Result<GuestAddress, X86Error> {
        let next_ip = if operand.rip_relative {
            self.next_ip()
        } else {
            0
        };
        operand_address(&*self.memory, &*self.state, operand, next_ip, access)
    }

    /// Read an operand.
    pub fn read(&mut self, operand: &X64Operand) -> Result<u64, X86Error> {
        let next_ip = match operand {
            X64Operand::Memory(memory) if memory.rip_relative => self.next_ip(),
            _ => 0,
        };
        read_operand(&mut *self.memory, &*self.state, operand, next_ip)
    }

    /// Write an operand.
    pub fn write(&mut self, operand: &X64Operand, value: u64) -> Result<(), X86Error> {
        let next_ip = match operand {
            X64Operand::Memory(memory) if memory.rip_relative => self.next_ip(),
            _ => 0,
        };
        write_operand(&mut *self.memory, &mut *self.state, operand, next_ip, value)
    }

    /// Pre-check a memory write.
    pub fn writable(&mut self, operand: &X64Operand) -> Result<(), X86Error> {
        let next_ip = match operand {
            X64Operand::Memory(memory) if memory.rip_relative => self.next_ip(),
            _ => 0,
        };
        writable_operand(&mut *self.memory, &*self.state, operand, next_ip)
    }
}
