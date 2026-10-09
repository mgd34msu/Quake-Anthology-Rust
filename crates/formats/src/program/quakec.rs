//! Version-six program tables and load-time operand lowering.
use crate::{FormatError, read::Reader};
use qa_core::{checksum::crc_block, names::NameTable, primitives::NameId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Opcode {
    Done,
    MulF,
    MulV,
    MulFV,
    MulVF,
    DivF,
    AddF,
    AddV,
    SubF,
    SubV,
    EqF,
    EqV,
    EqS,
    EqE,
    EqFnc,
    NeF,
    NeV,
    NeS,
    NeE,
    NeFnc,
    Le,
    Ge,
    Lt,
    Gt,
    LoadF,
    LoadV,
    LoadS,
    LoadEnt,
    LoadFld,
    LoadFnc,
    Address,
    StoreF,
    StoreV,
    StoreS,
    StoreEnt,
    StoreFld,
    StoreFnc,
    StorePF,
    StorePV,
    StorePS,
    StorePEnt,
    StorePFld,
    StorePFnc,
    Return,
    NotF,
    NotV,
    NotS,
    NotEnt,
    NotFnc,
    If,
    IfNot,
    Call0,
    Call1,
    Call2,
    Call3,
    Call4,
    Call5,
    Call6,
    Call7,
    Call8,
    State,
    Goto,
    And,
    Or,
    BitAnd,
    BitOr,
    Invalid = 255,
}
impl Opcode {
    pub fn decode(value: u16) -> Self {
        use Opcode::*;
        const OPS: [Opcode; 66] = [
            Done, MulF, MulV, MulFV, MulVF, DivF, AddF, AddV, SubF, SubV, EqF, EqV, EqS, EqE,
            EqFnc, NeF, NeV, NeS, NeE, NeFnc, Le, Ge, Lt, Gt, LoadF, LoadV, LoadS, LoadEnt,
            LoadFld, LoadFnc, Address, StoreF, StoreV, StoreS, StoreEnt, StoreFld, StoreFnc,
            StorePF, StorePV, StorePS, StorePEnt, StorePFld, StorePFnc, Return, NotF, NotV, NotS,
            NotEnt, NotFnc, If, IfNot, Call0, Call1, Call2, Call3, Call4, Call5, Call6, Call7,
            Call8, State, Goto, And, Or, BitAnd, BitOr,
        ];
        OPS.get(value as usize).copied().unwrap_or(Invalid)
    }
    fn widths(self) -> [usize; 3] {
        use Opcode::*;
        match self {
            Done | Return => [3, 0, 0],
            MulV | EqV | NeV => [3, 3, 1],
            MulFV => [1, 3, 3],
            MulVF => [3, 1, 3],
            AddV | SubV => [3, 3, 3],
            LoadV => [1, 1, 3],
            StoreV => [3, 3, 0],
            StorePV => [3, 1, 0],
            StoreF | StoreS | StoreEnt | StoreFld | StoreFnc | StorePF | StorePS | StorePEnt
            | StorePFld | StorePFnc => [1, 1, 0],
            NotV => [3, 0, 1],
            NotF | NotS | NotEnt | NotFnc => [1, 0, 1],
            If | IfNot | Call0 | Call1 | Call2 | Call3 | Call4 | Call5 | Call6 | Call7 | Call8 => {
                [1, 0, 0]
            }
            State => [1, 1, 0],
            Goto | Invalid => [0, 0, 0],
            _ => [1, 1, 1],
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Statement {
    pub opcode: Opcode,
    pub source_opcode: u16,
    pub operands: [i16; 3],
}
#[derive(Clone, Copy, Debug)]
pub struct Definition {
    pub value_type: u16,
    pub offset: usize,
    pub name: NameId,
}
#[derive(Clone, Copy, Debug)]
pub struct Function {
    pub first_statement: i32,
    pub parm_start: usize,
    pub locals: usize,
    pub name: NameId,
    pub file: NameId,
    pub numparms: usize,
    pub parm_size: [u8; 8],
}
pub struct Image {
    pub header_crc: i32,
    pub file_crc: u16,
    pub statements: Box<[Statement]>,
    pub globals: Box<[u32]>,
    pub globaldefs: Box<[Definition]>,
    pub fielddefs: Box<[Definition]>,
    pub functions: Box<[Function]>,
    pub strings: Box<[u8]>,
    pub names: NameTable,
    pub entityfields: usize,
    pub trapped_statements: usize,
}
fn string(strings: &[u8], offset: i32) -> Result<&[u8], FormatError> {
    let tail = strings
        .get(usize::try_from(offset).map_err(|_| FormatError::InvalidRange)?..)
        .ok_or(FormatError::InvalidRange)?;
    Ok(&tail[..tail
        .iter()
        .position(|&b| b == 0)
        .ok_or(FormatError::InvalidRange)?])
}
fn definition_width(kind: u16) -> Result<usize, FormatError> {
    match kind & 0x7fff {
        0..=7 => Ok(if kind & 0x7fff == 3 { 3 } else { 1 }),
        _ => Err(FormatError::Unsupported),
    }
}
impl Image {
    /// Expected header CRC is supplied by the module ABI. A mod can explicitly
    /// accept another layout; geometry or movement never selects this policy.
    pub fn parse(bytes: &[u8], expected_crc: Option<i32>) -> Result<Self, FormatError> {
        if bytes.len() > 256 * 1024 * 1024 {
            return Err(FormatError::InvalidRange);
        }
        let mut header = Reader::new(bytes);
        if header.i32()? != 6 {
            return Err(FormatError::Unsupported);
        }
        let header_crc = header.i32()?;
        if expected_crc.is_some_and(|c| c != header_crc) {
            return Err(FormatError::Checksum);
        }
        let mut sections = [(0usize, 0usize); 6];
        for s in &mut sections {
            s.0 = header.count(60, bytes.len())?;
            s.1 = header.count(0, bytes.len())?;
        }
        let entityfields = header.count(1, 1 << 20)?;
        let strides = [8, 8, 8, 36, 1, 4];
        let mut lumps = [&b""[..]; 6];
        for i in 0..6 {
            lumps[i] = header.section(sections[i].0, sections[i].1, strides[i], 60, bytes.len())?;
        }
        if sections[0].1 == 0
            || sections[3].1 == 0
            || sections[5].1 < 28
            || lumps[4].first() != Some(&0)
        {
            return Err(FormatError::InvalidRange);
        }
        for i in 0..6 {
            for j in i + 1..6 {
                if !lumps[i].is_empty()
                    && !lumps[j].is_empty()
                    && sections[i].0 < sections[j].0 + lumps[j].len()
                    && sections[j].0 < sections[i].0 + lumps[i].len()
                {
                    return Err(FormatError::InvalidRange);
                }
            }
        }
        let strings = lumps[4];
        let global_count = sections[5].1;
        let mut names = Vec::new();
        for index in [1, 2] {
            let mut r = Reader::new(lumps[index]);
            for _ in 0..sections[index].1 {
                let kind = r.u16()?;
                let offset = usize::from(r.u16()?);
                let name = r.i32()?;
                if index == 2 && kind & 0x8000 != 0 {
                    return Err(FormatError::InvalidValue);
                }
                let count = if index == 2 {
                    entityfields
                } else {
                    global_count
                };
                if offset
                    .checked_add(definition_width(kind)?)
                    .is_none_or(|end| end > count)
                {
                    return Err(FormatError::InvalidRange);
                }
                names.push(string(strings, name)?);
            }
        }
        let mut raw_functions = Vec::with_capacity(sections[3].1);
        let mut r = Reader::new(lumps[3]);
        for _ in 0..sections[3].1 {
            let first = r.i32()?;
            let parm_start = r.count(0, global_count)?;
            let locals = r.count(0, global_count)?;
            r.i32()?;
            let name = r.i32()?;
            let file = r.i32()?;
            let numparms = r.count(0, 8)?;
            let mut sizes = [0u8; 8];
            sizes.copy_from_slice(r.take(8)?);
            if parm_start
                .checked_add(locals)
                .is_none_or(|end| end > global_count)
                || sizes[..numparms].iter().any(|&s| s > 3)
                || parm_start
                    .checked_add(
                        sizes[..numparms]
                            .iter()
                            .map(|&s| usize::from(s))
                            .sum::<usize>(),
                    )
                    .is_none_or(|end| end > global_count)
            {
                return Err(FormatError::InvalidRange);
            }
            names.push(string(strings, name)?);
            names.push(string(strings, file)?);
            raw_functions.push((first, parm_start, locals, name, file, numparms, sizes));
        }
        let names = NameTable::load(names).map_err(|_| FormatError::InvalidRange)?;
        let interned = |offset| {
            names
                .find(string(strings, offset)?)
                .ok_or(FormatError::InvalidValue)
        };
        let mut definitions = [Vec::new(), Vec::new()];
        for index in [1, 2] {
            let mut r = Reader::new(lumps[index]);
            for _ in 0..sections[index].1 {
                definitions[index - 1].push(Definition {
                    value_type: r.u16()?,
                    offset: usize::from(r.u16()?),
                    name: interned(r.i32()?)?,
                });
            }
        }
        let functions = raw_functions
            .into_iter()
            .map(
                |(first_statement, parm_start, locals, name, file, numparms, parm_size)| {
                    Ok(Function {
                        first_statement,
                        parm_start,
                        locals,
                        name: interned(name)?,
                        file: interned(file)?,
                        numparms,
                        parm_size,
                    })
                },
            )
            .collect::<Result<Box<[_]>, FormatError>>()?;
        let mut statements = Vec::with_capacity(sections[0].1);
        let mut r = Reader::new(lumps[0]);
        let mut trapped_statements = 0;
        for index in 0..sections[0].1 {
            let source_opcode = r.u16()?;
            let mut opcode = Opcode::decode(source_opcode);
            let operands = [r.i16()?, r.i16()?, r.i16()?];
            for (width, value) in opcode.widths().into_iter().zip(operands) {
                if width > 0 && (value < 0 || value as usize + width > global_count) {
                    opcode = Opcode::Invalid;
                }
            }
            let relative = match opcode {
                Opcode::If | Opcode::IfNot => Some(operands[1]),
                Opcode::Goto => Some(operands[0]),
                _ => None,
            };
            if relative.is_some_and(|delta| {
                index as i64 + i64::from(delta) < 0
                    || index as i64 + i64::from(delta) >= sections[0].1 as i64
            }) {
                opcode = Opcode::Invalid;
            }
            if opcode == Opcode::Invalid {
                trapped_statements += 1;
            }
            statements.push(Statement {
                opcode,
                source_opcode,
                operands,
            });
        }
        let mut r = Reader::new(lumps[5]);
        let globals = (0..global_count)
            .map(|_| r.u32())
            .collect::<Result<Box<[_]>, _>>()?;
        let [globaldefs, fielddefs] = definitions;
        Ok(Self {
            header_crc,
            file_crc: crc_block(bytes),
            statements: statements.into_boxed_slice(),
            globals,
            globaldefs: globaldefs.into_boxed_slice(),
            fielddefs: fielddefs.into_boxed_slice(),
            functions,
            strings: strings.into(),
            names,
            entityfields,
            trapped_statements,
        })
    }
    pub fn global(&self, name: &[u8]) -> Option<usize> {
        let name = self.names.find(name)?;
        self.globaldefs
            .iter()
            .find(|d| d.name == name)
            .map(|d| d.offset)
    }
    pub fn field(&self, name: &[u8]) -> Option<usize> {
        let name = self.names.find(name)?;
        self.fielddefs
            .iter()
            .find(|d| d.name == name)
            .map(|d| d.offset)
    }
    pub fn function(&self, name: &[u8]) -> Option<u32> {
        let name = self.names.find(name)?;
        self.functions
            .iter()
            .position(|f| f.name == name)
            .map(|i| i as u32)
    }
}
