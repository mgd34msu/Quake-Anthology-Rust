//! qsrc qcommon/vm.c header and vm_interpreted.c instruction encoding.
use crate::{FormatError, read::Reader};

pub const MAGIC: u32 = 0x12721444;
const MAX_MEMORY: usize = 512 * 1024 * 1024;
const MAX_CODE: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Opcode {
    Ignore = 1,
    Break,
    Enter,
    Leave,
    Call,
    Push,
    Pop,
    Const,
    Local,
    Jump,
    Eq,
    Ne,
    LtI,
    LeI,
    GtI,
    GeI,
    LtU,
    LeU,
    GtU,
    GeU,
    EqF,
    NeF,
    LtF,
    LeF,
    GtF,
    GeF,
    Load1,
    Load2,
    Load4,
    Store1,
    Store2,
    Store4,
    Arg,
    BlockCopy,
    Sex8,
    Sex16,
    NegI,
    Add,
    Sub,
    DivI,
    DivU,
    ModI,
    ModU,
    MulI,
    MulU,
    Band,
    Bor,
    Bxor,
    Bcom,
    Lsh,
    RshI,
    RshU,
    NegF,
    AddF,
    SubF,
    DivF,
    MulF,
    CvIF,
    CvFI,
}
impl Opcode {
    pub fn decode(value: u8) -> Result<Self, FormatError> {
        use Opcode::*;
        const OPS: [Opcode; 59] = [
            Ignore, Break, Enter, Leave, Call, Push, Pop, Const, Local, Jump, Eq, Ne, LtI, LeI,
            GtI, GeI, LtU, LeU, GtU, GeU, EqF, NeF, LtF, LeF, GtF, GeF, Load1, Load2, Load4,
            Store1, Store2, Store4, Arg, BlockCopy, Sex8, Sex16, NegI, Add, Sub, DivI, DivU, ModI,
            ModU, MulI, MulU, Band, Bor, Bxor, Bcom, Lsh, RshI, RshU, NegF, AddF, SubF, DivF, MulF,
            CvIF, CvFI,
        ];
        OPS.get(value.wrapping_sub(1) as usize)
            .copied()
            .ok_or(FormatError::InvalidValue)
    }
    pub fn operand_bytes(self) -> usize {
        match self {
            Self::Arg => 1,
            Self::Enter | Self::Leave | Self::Const | Self::Local | Self::BlockCopy => 4,
            value if value.branch() => 4,
            _ => 0,
        }
    }
    pub fn branch(self) -> bool {
        (Self::Eq as u8..=Self::GeF as u8).contains(&(self as u8))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Instruction {
    pub opcode: Opcode,
    pub operand: i32,
    /// Native saved return PCs are byte offsets, while branch operands and
    /// function values are instruction ordinals. Keep both exact namespaces.
    pub byte_pc: u32,
    pub next_byte_pc: u32,
}

pub struct Image {
    pub instructions: Box<[Instruction]>,
    pub byte_to_instruction: Box<[u32]>,
    pub initialized: Box<[u8]>,
    pub memory_size: usize,
    pub data_length: usize,
    pub literal_length: usize,
    pub bss_length: usize,
}

fn section(bytes: &[u8], offset: i32, count: usize) -> Result<&[u8], FormatError> {
    let offset = usize::try_from(offset).map_err(|_| FormatError::InvalidRange)?;
    let end = offset.checked_add(count).ok_or(FormatError::InvalidRange)?;
    bytes.get(offset..end).ok_or(FormatError::InvalidRange)
}

impl Image {
    pub fn parse(bytes: &[u8]) -> Result<Self, FormatError> {
        let mut header = Reader::new(bytes);
        if header.u32()? != MAGIC {
            return Err(FormatError::Unsupported);
        }
        let count = header.count(1, MAX_CODE)?;
        let code_offset = header.i32()?;
        let code_length = header.count(count, MAX_CODE)?;
        let data_offset = header.i32()?;
        let data_length = header.count(0, MAX_MEMORY)?;
        let literal_length = header.count(0, MAX_MEMORY)?;
        let bss_length = header.count(0, MAX_MEMORY)?;
        if code_offset < 32 || data_offset < 32 || data_length % 4 != 0 {
            return Err(FormatError::InvalidRange);
        }
        let initialized_length = data_length
            .checked_add(literal_length)
            .ok_or(FormatError::InvalidRange)?;
        let total = initialized_length
            .checked_add(bss_length)
            .ok_or(FormatError::InvalidRange)?;
        let memory_size = total
            .max(1)
            .checked_next_power_of_two()
            .filter(|&size| size <= MAX_MEMORY)
            .ok_or(FormatError::InvalidRange)?;
        let encoded = section(bytes, code_offset, code_length)?;
        let initialized = section(bytes, data_offset, initialized_length)?;
        if initialized_length > 0
            && (code_offset as usize) < data_offset as usize + initialized_length
            && (data_offset as usize) < code_offset as usize + code_length
        {
            return Err(FormatError::InvalidRange);
        }
        let mut instructions = Vec::with_capacity(count);
        let mut code = Reader::new(encoded);
        let mut byte_to_instruction = vec![u32::MAX; code_length].into_boxed_slice();
        for index in 0..count {
            let byte_pc = code.at;
            let opcode = Opcode::decode(code.u8()?)?;
            let operand = match opcode.operand_bytes() {
                1 => i32::from(code.u8()?),
                4 => code.i32()?,
                _ => 0,
            };
            if opcode.branch() && (operand < 0 || operand as usize >= count) {
                return Err(FormatError::InvalidReference("QVM branch", index));
            }
            if matches!(opcode, Opcode::Enter | Opcode::Leave | Opcode::BlockCopy)
                && (operand < 0 || operand & 3 != 0)
            {
                return Err(FormatError::InvalidValue);
            }
            if opcode == Opcode::Arg && operand & 3 != 0 {
                return Err(FormatError::InvalidValue);
            }
            byte_to_instruction[byte_pc] = index as u32;
            instructions.push(Instruction {
                opcode,
                operand,
                byte_pc: byte_pc as u32,
                next_byte_pc: code.at as u32,
            });
        }
        // q3asm rounds every segment up to four bytes after counting its
        // instructions. Unaddressable code tail bytes are not instructions.
        Ok(Self {
            instructions: instructions.into_boxed_slice(),
            byte_to_instruction,
            initialized: initialized.into(),
            memory_size,
            data_length,
            literal_length,
            bss_length,
        })
    }
}
