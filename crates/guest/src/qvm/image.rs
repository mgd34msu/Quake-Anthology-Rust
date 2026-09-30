//! QVM image decoding: header checks and instruction tables.
//!
//! Port of `src/compat/qvm/image.ts` (translated from Quake III Arena's
//! `qfiles.h`, `vm_local.h`, `vm.c` `VM_Create`/`VM_Restart` and
//! `vm_interpreted.c` `VM_PrepareInterpreter`; Copyright (C) 1999-2005
//! Id Software, Inc., GPL-2.0-or-later).
//!
//! Decodes the pinned 1.32b format. Branch operands stay instruction-table
//! indices here (the interpreter rewrites them to byte offsets when preparing
//! code). Failures use [`GuestError::BadImage`](crate::error::GuestError).

use crate::error::GuestError;

/// QVM magic from `qfiles.h`.
pub const QVM_MAGIC: u32 = 0x1272_1444;
/// Header length in bytes: eight little-endian words.
pub const QVM_HEADER_LENGTH: usize = 32;
/// `OP_ARG` encodes a byte offset; aligned words occupy caller offsets 8..=252.
pub const QVM_MAX_PRIVATE_ARGUMENT_WORDS: usize = 62;
/// Largest data allocation: positive signed range for the power-of-two loop.
pub const QVM_MAX_DATA_LENGTH: usize = 0x4000_0000;
/// Largest code section the source allocator accepts.
pub const QVM_MAX_CODE_LENGTH: usize = 0x1FFF_FFFF;

/// QVM operation codes in `qfiles.h` order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum QvmOpcode {
    /// Undefined instruction.
    OpUndef = 0,
    /// No-op.
    OpIgnore = 1,
    /// Debugger break.
    OpBreak = 2,
    /// Enter a function frame.
    OpEnter = 3,
    /// Leave a function frame.
    OpLeave = 4,
    /// Call a function or trap.
    OpCall = 5,
    /// Reserve an operand slot.
    OpPush = 6,
    /// Drop the top operand.
    OpPop = 7,
    /// Push a constant word.
    OpConst = 8,
    /// Push a local address.
    OpLocal = 9,
    /// Unconditional jump.
    OpJump = 10,
    /// Branch if equal.
    OpEq = 11,
    /// Branch if not equal.
    OpNe = 12,
    /// Branch if signed less than.
    OpLti = 13,
    /// Branch if signed less than or equal.
    OpLei = 14,
    /// Branch if signed greater than.
    OpGti = 15,
    /// Branch if signed greater than or equal.
    OpGei = 16,
    /// Branch if unsigned less than.
    OpLtu = 17,
    /// Branch if unsigned less than or equal.
    OpLeu = 18,
    /// Branch if unsigned greater than.
    OpGtu = 19,
    /// Branch if unsigned greater than or equal.
    OpGeu = 20,
    /// Branch if floats equal.
    OpEqf = 21,
    /// Branch if floats differ.
    OpNef = 22,
    /// Branch if float less than.
    OpLtf = 23,
    /// Branch if float less than or equal.
    OpLef = 24,
    /// Branch if float greater than.
    OpGtf = 25,
    /// Branch if float greater than or equal.
    OpGef = 26,
    /// Load one byte.
    OpLoad1 = 27,
    /// Load two bytes.
    OpLoad2 = 28,
    /// Load four bytes.
    OpLoad4 = 29,
    /// Store one byte.
    OpStore1 = 30,
    /// Store two bytes.
    OpStore2 = 31,
    /// Store four bytes.
    OpStore4 = 32,
    /// Publish an argument word.
    OpArg = 33,
    /// Copy a memory block.
    OpBlockCopy = 34,
    /// Sign-extend a byte.
    OpSex8 = 35,
    /// Sign-extend a half word.
    OpSex16 = 36,
    /// Integer negation.
    OpNegi = 37,
    /// Integer addition.
    OpAdd = 38,
    /// Integer subtraction.
    OpSub = 39,
    /// Signed division.
    OpDivi = 40,
    /// Unsigned division.
    OpDivu = 41,
    /// Signed modulo.
    OpModi = 42,
    /// Unsigned modulo.
    OpModu = 43,
    /// Signed multiplication.
    OpMuli = 44,
    /// Unsigned multiplication.
    OpMulu = 45,
    /// Bitwise and.
    OpBand = 46,
    /// Bitwise or.
    OpBor = 47,
    /// Bitwise exclusive or.
    OpBxor = 48,
    /// Bitwise complement.
    OpBcom = 49,
    /// Logical shift left.
    OpLsh = 50,
    /// Arithmetic shift right.
    OpRshi = 51,
    /// Logical shift right.
    OpRshu = 52,
    /// Float negation.
    OpNegf = 53,
    /// Float addition.
    OpAddf = 54,
    /// Float subtraction.
    OpSubf = 55,
    /// Float division.
    OpDivf = 56,
    /// Float multiplication.
    OpMulf = 57,
    /// Convert integer to float.
    OpCvif = 58,
    /// Convert float to integer.
    OpCvfi = 59,
}

impl QvmOpcode {
    /// Decode one opcode byte.
    pub fn from_u8(byte: u8) -> Result<Self, GuestError> {
        match byte {
            0 => Ok(Self::OpUndef),
            1 => Ok(Self::OpIgnore),
            2 => Ok(Self::OpBreak),
            3 => Ok(Self::OpEnter),
            4 => Ok(Self::OpLeave),
            5 => Ok(Self::OpCall),
            6 => Ok(Self::OpPush),
            7 => Ok(Self::OpPop),
            8 => Ok(Self::OpConst),
            9 => Ok(Self::OpLocal),
            10 => Ok(Self::OpJump),
            11 => Ok(Self::OpEq),
            12 => Ok(Self::OpNe),
            13 => Ok(Self::OpLti),
            14 => Ok(Self::OpLei),
            15 => Ok(Self::OpGti),
            16 => Ok(Self::OpGei),
            17 => Ok(Self::OpLtu),
            18 => Ok(Self::OpLeu),
            19 => Ok(Self::OpGtu),
            20 => Ok(Self::OpGeu),
            21 => Ok(Self::OpEqf),
            22 => Ok(Self::OpNef),
            23 => Ok(Self::OpLtf),
            24 => Ok(Self::OpLef),
            25 => Ok(Self::OpGtf),
            26 => Ok(Self::OpGef),
            27 => Ok(Self::OpLoad1),
            28 => Ok(Self::OpLoad2),
            29 => Ok(Self::OpLoad4),
            30 => Ok(Self::OpStore1),
            31 => Ok(Self::OpStore2),
            32 => Ok(Self::OpStore4),
            33 => Ok(Self::OpArg),
            34 => Ok(Self::OpBlockCopy),
            35 => Ok(Self::OpSex8),
            36 => Ok(Self::OpSex16),
            37 => Ok(Self::OpNegi),
            38 => Ok(Self::OpAdd),
            39 => Ok(Self::OpSub),
            40 => Ok(Self::OpDivi),
            41 => Ok(Self::OpDivu),
            42 => Ok(Self::OpModi),
            43 => Ok(Self::OpModu),
            44 => Ok(Self::OpMuli),
            45 => Ok(Self::OpMulu),
            46 => Ok(Self::OpBand),
            47 => Ok(Self::OpBor),
            48 => Ok(Self::OpBxor),
            49 => Ok(Self::OpBcom),
            50 => Ok(Self::OpLsh),
            51 => Ok(Self::OpRshi),
            52 => Ok(Self::OpRshu),
            53 => Ok(Self::OpNegf),
            54 => Ok(Self::OpAddf),
            55 => Ok(Self::OpSubf),
            56 => Ok(Self::OpDivf),
            57 => Ok(Self::OpMulf),
            58 => Ok(Self::OpCvif),
            59 => Ok(Self::OpCvfi),
            other => Err(GuestError::bad_image("qvm", format!("unknown QVM opcode {other}"))),
        }
    }

    /// Whether the opcode is a conditional branch.
    #[must_use]
    pub fn is_branch(self) -> bool {
        matches!(
            self,
            Self::OpEq
                | Self::OpNe
                | Self::OpLti
                | Self::OpLei
                | Self::OpGti
                | Self::OpGei
                | Self::OpLtu
                | Self::OpLeu
                | Self::OpGtu
                | Self::OpGeu
                | Self::OpEqf
                | Self::OpNef
                | Self::OpLtf
                | Self::OpLef
                | Self::OpGtf
                | Self::OpGef
        )
    }

    /// Operand width in code bytes: 4 for word operands, 1 for `OP_ARG`, else 0.
    #[must_use]
    pub fn operand_width(self) -> usize {
        if self == Self::OpArg {
            1
        } else if self.is_branch()
            || matches!(
                self,
                Self::OpEnter | Self::OpLeave | Self::OpConst | Self::OpLocal | Self::OpBlockCopy
            )
        {
            4
        } else {
            0
        }
    }

    /// Source `opnames` spelling (for example `"OP_ENTER"`).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::OpUndef => "OP_UNDEF",
            Self::OpIgnore => "OP_IGNORE",
            Self::OpBreak => "OP_BREAK",
            Self::OpEnter => "OP_ENTER",
            Self::OpLeave => "OP_LEAVE",
            Self::OpCall => "OP_CALL",
            Self::OpPush => "OP_PUSH",
            Self::OpPop => "OP_POP",
            Self::OpConst => "OP_CONST",
            Self::OpLocal => "OP_LOCAL",
            Self::OpJump => "OP_JUMP",
            Self::OpEq => "OP_EQ",
            Self::OpNe => "OP_NE",
            Self::OpLti => "OP_LTI",
            Self::OpLei => "OP_LEI",
            Self::OpGti => "OP_GTI",
            Self::OpGei => "OP_GEI",
            Self::OpLtu => "OP_LTU",
            Self::OpLeu => "OP_LEU",
            Self::OpGtu => "OP_GTU",
            Self::OpGeu => "OP_GEU",
            Self::OpEqf => "OP_EQF",
            Self::OpNef => "OP_NEF",
            Self::OpLtf => "OP_LTF",
            Self::OpLef => "OP_LEF",
            Self::OpGtf => "OP_GTF",
            Self::OpGef => "OP_GEF",
            Self::OpLoad1 => "OP_LOAD1",
            Self::OpLoad2 => "OP_LOAD2",
            Self::OpLoad4 => "OP_LOAD4",
            Self::OpStore1 => "OP_STORE1",
            Self::OpStore2 => "OP_STORE2",
            Self::OpStore4 => "OP_STORE4",
            Self::OpArg => "OP_ARG",
            Self::OpBlockCopy => "OP_BLOCK_COPY",
            Self::OpSex8 => "OP_SEX8",
            Self::OpSex16 => "OP_SEX16",
            Self::OpNegi => "OP_NEGI",
            Self::OpAdd => "OP_ADD",
            Self::OpSub => "OP_SUB",
            Self::OpDivi => "OP_DIVI",
            Self::OpDivu => "OP_DIVU",
            Self::OpModi => "OP_MODI",
            Self::OpModu => "OP_MODU",
            Self::OpMuli => "OP_MULI",
            Self::OpMulu => "OP_MULU",
            Self::OpBand => "OP_BAND",
            Self::OpBor => "OP_BOR",
            Self::OpBxor => "OP_BXOR",
            Self::OpBcom => "OP_BCOM",
            Self::OpLsh => "OP_LSH",
            Self::OpRshi => "OP_RSHI",
            Self::OpRshu => "OP_RSHU",
            Self::OpNegf => "OP_NEGF",
            Self::OpAddf => "OP_ADDF",
            Self::OpSubf => "OP_SUBF",
            Self::OpDivf => "OP_DIVF",
            Self::OpMulf => "OP_MULF",
            Self::OpCvif => "OP_CVIF",
            Self::OpCvfi => "OP_CVFI",
        }
    }
}

/// Decoded instruction operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmOperand {
    /// Single-byte opcode with no operand.
    None,
    /// Four-byte signed word operand.
    Word(i32),
    /// One-byte `OP_ARG` operand.
    Byte(u8),
}

/// One decoded instruction: its code-section byte offset plus opcode/operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmInstruction {
    /// Offset from the code start; also the source interpreter's program counter.
    pub byte_offset: usize,
    /// Operation code.
    pub opcode: QvmOpcode,
    /// Decoded operand.
    pub operand: QvmOperand,
}

impl QvmInstruction {
    /// Operand width in code bytes.
    #[must_use]
    pub fn operand_width(&self) -> usize {
        self.opcode.operand_width()
    }
}

/// Initialized data plus the allocation length (the parser allocates nothing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmDataImage {
    /// Owned initialized data followed by literals, little-endian words intact.
    pub initialized_data: Vec<u8>,
    /// Power-of-two allocation length covering data, literals, BSS and padding.
    pub allocated_data_length: usize,
}

/// Fully decoded QVM image: instructions plus section lengths.
#[derive(Debug, Clone)]
pub struct QvmImage {
    /// Image source label (for diagnostics).
    pub source: String,
    /// Decoded instruction table.
    pub instructions: Vec<QvmInstruction>,
    /// Code section file offset.
    pub code_offset: usize,
    /// Code section length in bytes.
    pub code_length: usize,
    /// Initialized data length in bytes.
    pub data_length: usize,
    /// Literal length in bytes.
    pub literal_length: usize,
    /// BSS length in bytes.
    pub bss_length: usize,
    /// Owned initialized data followed by literals.
    pub initialized_data: Vec<u8>,
    /// Power-of-two allocation length covering data, literals, BSS and padding.
    pub allocated_data_length: usize,
    /// Data mask (`allocated_data_length - 1`).
    pub data_mask: i32,
}

/// Source header fields shared by `VM_Create` and `VM_Restart` checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmSourceHeader {
    /// Data section file offset.
    pub data_offset: usize,
    /// Initialized data length in bytes.
    pub data_length: usize,
    /// Literal length in bytes.
    pub literal_length: usize,
    /// BSS length in bytes.
    pub bss_length: usize,
}

fn bad(source: &str, offset: usize, detail: &str) -> GuestError {
    GuestError::bad_image("qvm", format!("{source}@{offset}: {detail}"))
}

fn read_i32(bytes: &[u8], offset: usize, source: &str) -> Result<i32, GuestError> {
    if offset > bytes.len() || 4 > bytes.len() - offset {
        return Err(bad(source, offset, "QVM header exceeds file"));
    }
    Ok(i32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ]))
}

/// Shared `VM_Create`/`VM_Restart` header checks before allocating or copying.
pub fn read_qvm_source_header(bytes: &[u8], source: &str) -> Result<QvmSourceHeader, GuestError> {
    let magic = read_i32(bytes, 0, source)?;
    let code_length = read_i32(bytes, 12, source)?;
    let data_offset = read_i32(bytes, 16, source)?;
    let data_length = read_i32(bytes, 20, source)?;
    let literal_length = read_i32(bytes, 24, source)?;
    let bss_length = read_i32(bytes, 28, source)?;
    if magic as u32 != QVM_MAGIC || bss_length < 0 || data_length < 0 || literal_length < 0 || code_length <= 0 {
        return Err(GuestError::bad_image("qvm", format!("{source} has bad header")));
    }
    Ok(QvmSourceHeader {
        data_offset: data_offset as usize,
        data_length: data_length as usize,
        literal_length: literal_length as usize,
        bss_length: bss_length as usize,
    })
}

/// `VM_Restart` data: prepared code is retained, the instruction table ignored.
pub fn parse_qvm_restart(bytes: &[u8], source: &str) -> Result<QvmDataImage, GuestError> {
    let header = read_qvm_source_header(bytes, source)?;
    let initialized_length = header.data_length + header.literal_length;
    let total = initialized_length + header.bss_length;
    if total > QVM_MAX_DATA_LENGTH {
        return Err(bad(
            source,
            28,
            "QVM data exceeds positive signed power-of-two allocation range",
        ));
    }
    let mut allocated = 1usize;
    while allocated < total {
        allocated *= 2;
    }
    if header.data_offset > bytes.len() || initialized_length > bytes.len() - header.data_offset {
        return Err(bad(source, header.data_offset, "QVM data exceeds file"));
    }
    Ok(QvmDataImage {
        initialized_data: bytes[header.data_offset..header.data_offset + initialized_length].to_vec(),
        allocated_data_length: allocated,
    })
}

fn check_section(
    total: usize,
    offset: usize,
    length: usize,
    field_offset: usize,
    source: &str,
) -> Result<(), GuestError> {
    if offset < QVM_HEADER_LENGTH || offset > total || length > total - offset {
        return Err(bad(
            source,
            field_offset,
            &format!("invalid QVM section at {offset} with length {length}"),
        ));
    }
    Ok(())
}

fn decode_instruction(
    bytes: &[u8],
    cursor: &mut usize,
    code_offset: usize,
    code_end: usize,
    instruction_count: usize,
    source: &str,
) -> Result<QvmInstruction, GuestError> {
    let byte_offset = *cursor - code_offset;
    if *cursor >= code_end {
        return Err(bad(source, *cursor, "QVM instruction exceeds code section"));
    }
    let opcode = QvmOpcode::from_u8(bytes[*cursor])
        .map_err(|_| bad(source, *cursor, &format!("unknown QVM opcode {}", bytes[*cursor])))?;
    *cursor += 1;
    if opcode.is_branch() {
        if *cursor > code_end || 4 > code_end - *cursor {
            return Err(bad(source, *cursor, "QVM word operand exceeds code section"));
        }
        let operand = i32::from_le_bytes([
            bytes[*cursor],
            bytes[*cursor + 1],
            bytes[*cursor + 2],
            bytes[*cursor + 3],
        ]);
        // Source preparation rewrites these indices to byte offsets. Keep indices
        // here; CALL/JUMP get their dynamic indices from the operand stack instead.
        if operand < 0 || operand as usize >= instruction_count {
            return Err(bad(
                source,
                *cursor,
                &format!("QVM branch target {operand} outside instruction table"),
            ));
        }
        *cursor += 4;
        return Ok(QvmInstruction {
            byte_offset,
            opcode,
            operand: QvmOperand::Word(operand),
        });
    }
    match opcode {
        QvmOpcode::OpEnter | QvmOpcode::OpLeave | QvmOpcode::OpConst | QvmOpcode::OpLocal | QvmOpcode::OpBlockCopy => {
            if *cursor > code_end || 4 > code_end - *cursor {
                return Err(bad(source, *cursor, "QVM word operand exceeds code section"));
            }
            let operand = i32::from_le_bytes([
                bytes[*cursor],
                bytes[*cursor + 1],
                bytes[*cursor + 2],
                bytes[*cursor + 3],
            ]);
            *cursor += 4;
            Ok(QvmInstruction {
                byte_offset,
                opcode,
                operand: QvmOperand::Word(operand),
            })
        }
        QvmOpcode::OpArg => {
            if *cursor >= code_end {
                return Err(bad(source, *cursor, "QVM byte operand exceeds code section"));
            }
            let operand = bytes[*cursor];
            *cursor += 1;
            Ok(QvmInstruction {
                byte_offset,
                opcode,
                operand: QvmOperand::Byte(operand),
            })
        }
        _ => Ok(QvmInstruction {
            byte_offset,
            opcode,
            operand: QvmOperand::None,
        }),
    }
}

/// Decode the pinned 1.32b format. Execution belongs to the VM owner.
pub fn parse_qvm(bytes: &[u8], source: &str) -> Result<QvmImage, GuestError> {
    if bytes.len() < QVM_HEADER_LENGTH {
        return Err(bad(source, 0, "invalid QVM magic"));
    }
    let magic = read_i32(bytes, 0, source)?;
    if magic as u32 != QVM_MAGIC {
        return Err(bad(source, 0, "invalid QVM magic"));
    }
    let instruction_count = read_i32(bytes, 4, source)?;
    let code_offset = read_i32(bytes, 8, source)?;
    let code_length = read_i32(bytes, 12, source)?;
    let data_offset = read_i32(bytes, 16, source)?;
    let data_length = read_i32(bytes, 20, source)?;
    let literal_length = read_i32(bytes, 24, source)?;
    let bss_length = read_i32(bytes, 28, source)?;
    if code_length <= 0 || code_length as usize > QVM_MAX_CODE_LENGTH {
        return Err(bad(source, 12, "QVM code length outside source allocation range"));
    }
    if instruction_count <= 0 || instruction_count > code_length {
        return Err(bad(source, 4, "QVM instruction count outside code bounds"));
    }
    if data_length < 0 || data_length % 4 != 0 {
        return Err(bad(source, 20, "QVM data length must contain complete words"));
    }
    if literal_length < 0 {
        return Err(bad(source, 24, "negative QVM literal length"));
    }
    if bss_length < 0 {
        return Err(bad(source, 28, "negative QVM BSS length"));
    }
    let code_offset = code_offset as usize;
    let code_length = code_length as usize;
    let data_offset = data_offset as usize;
    let data_length = data_length as usize;
    let literal_length = literal_length as usize;
    let bss_length = bss_length as usize;
    let initialized_length = data_length + literal_length;
    let total = initialized_length + bss_length;
    if total > QVM_MAX_DATA_LENGTH {
        return Err(bad(
            source,
            28,
            "QVM data exceeds positive signed power-of-two allocation range",
        ));
    }
    check_section(bytes.len(), code_offset, code_length, 8, source)?;
    check_section(bytes.len(), data_offset, initialized_length, 16, source)?;
    if initialized_length > 0
        && code_offset < data_offset + initialized_length
        && data_offset < code_offset + code_length
    {
        return Err(bad(source, 16, "QVM code and initialized data sections overlap"));
    }
    let mut allocated = 1usize;
    while allocated < total {
        allocated *= 2;
    }
    let code_end = code_offset + code_length;
    let mut cursor = code_offset;
    let mut instructions = Vec::with_capacity(instruction_count as usize);
    for _ in 0..instruction_count {
        instructions.push(decode_instruction(
            bytes,
            &mut cursor,
            code_offset,
            code_end,
            instruction_count as usize,
            source,
        )?);
    }
    // q3asm aligns the code section to four bytes. Like VM_PrepareInterpreter,
    // decode only instructionCount instructions, leaving the rest inert.
    Ok(QvmImage {
        source: source.to_string(),
        instructions,
        code_offset,
        code_length,
        data_length,
        literal_length,
        bss_length,
        initialized_data: bytes[data_offset..data_offset + initialized_length].to_vec(),
        allocated_data_length: allocated,
        data_mask: (allocated - 1) as i32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(instruction_count: i32, code: &[u8], data: &[u8]) -> Vec<u8> {
        let code_offset = QVM_HEADER_LENGTH as i32;
        let mut code_padded = code.to_vec();
        while !code_padded.len().is_multiple_of(4) {
            code_padded.push(0);
        }
        let data_offset = code_offset + code_padded.len() as i32;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(QVM_MAGIC as i32).to_le_bytes());
        bytes.extend_from_slice(&instruction_count.to_le_bytes());
        bytes.extend_from_slice(&code_offset.to_le_bytes());
        bytes.extend_from_slice(&(code_padded.len() as i32).to_le_bytes());
        bytes.extend_from_slice(&data_offset.to_le_bytes());
        bytes.extend_from_slice(&(data.len() as i32).to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&code_padded);
        bytes.extend_from_slice(data);
        bytes
    }

    #[test]
    fn decodes_enter_const_leave() {
        // ENTER 8; CONST 7; LEAVE 8
        let mut code = vec![QvmOpcode::OpEnter as u8];
        code.extend_from_slice(&8i32.to_le_bytes());
        code.push(QvmOpcode::OpConst as u8);
        code.extend_from_slice(&7i32.to_le_bytes());
        code.push(QvmOpcode::OpLeave as u8);
        code.extend_from_slice(&8i32.to_le_bytes());
        let image = parse_qvm(&header(3, &code, &[1, 2, 3, 4]), "test").unwrap();
        assert_eq!(image.instructions.len(), 3);
        assert_eq!(image.instructions[1].operand, QvmOperand::Word(7));
        assert_eq!(image.instructions[2].byte_offset, 10);
        assert_eq!(image.allocated_data_length, 4);
        assert_eq!(image.data_mask, 3);
        assert_eq!(image.initialized_data, vec![1, 2, 3, 4]);
    }

    #[test]
    fn branch_targets_are_validated_as_indices() {
        let mut code = vec![QvmOpcode::OpConst as u8];
        code.extend_from_slice(&0i32.to_le_bytes());
        code.push(QvmOpcode::OpEq as u8);
        code.extend_from_slice(&9i32.to_le_bytes());
        assert!(parse_qvm(&header(2, &code, &[]), "test").is_err());
    }

    #[test]
    fn rejects_bad_magic_and_overlaps() {
        let bad = header(1, &[QvmOpcode::OpBreak as u8], &[]);
        let mut bytes = bad.clone();
        bytes[0] = 0;
        assert!(parse_qvm(&bytes, "test").is_err());
        // Truncated file.
        assert!(parse_qvm(&[0u8; 8], "test").is_err());
    }

    #[test]
    fn restart_keeps_initialized_data() {
        let code = [QvmOpcode::OpBreak as u8];
        let bytes = header(1, &code, &[9, 9, 9, 9]);
        let data = parse_qvm_restart(&bytes, "test").unwrap();
        assert_eq!(data.initialized_data, vec![9, 9, 9, 9]);
        assert_eq!(data.allocated_data_length, 4);
    }

    #[test]
    fn opcode_names_cover_debug_traces() {
        assert_eq!(QvmOpcode::OpEnter.name(), "OP_ENTER");
        assert_eq!(QvmOpcode::OpCvfi.name(), "OP_CVFI");
        assert!(QvmOpcode::OpGef.is_branch());
        assert!(!QvmOpcode::OpCall.is_branch());
        assert_eq!(QvmOpcode::OpConst.operand_width(), 4);
        assert_eq!(QvmOpcode::OpArg.operand_width(), 1);
        assert_eq!(QvmOpcode::OpAdd.operand_width(), 0);
        assert!(QvmOpcode::from_u8(60).is_err());
    }
}
