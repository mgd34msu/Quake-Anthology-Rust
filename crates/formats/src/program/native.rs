//! Inert native module images over one virtual-byte/symbol representation.
use crate::{FormatError, read::Reader};
use qa_core::{names::NameTable, primitives::NameId};

mod elf;
mod pe;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadRole {
    Library,
    Program,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Pe,
    Elf,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub bits: u32,
    pub encoding: Encoding,
}
#[derive(Clone, Copy, Debug)]
pub struct Region {
    pub offset: usize,
    pub length: usize,
    pub read: bool,
    pub write: bool,
    pub execute: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct Version {
    pub name: NameId,
    pub library: Option<NameId>,
    pub weak: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct Symbol {
    pub name: Option<NameId>,
    pub address: u64,
    pub bytes: u64,
    pub ordinal: Option<u32>,
    pub forward: Option<NameId>,
    pub defined: bool,
    pub absolute: bool,
    pub weak: bool,
    pub section: u32,
    pub binding: u8,
    pub kind: u8,
    pub visibility: u8,
    pub version: Option<Version>,
    pub hidden_version: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct Import {
    pub library: Option<NameId>,
    pub name: Option<NameId>,
    pub ordinal: Option<u16>,
    pub slot: u64,
    pub delayed: bool,
    pub hint: Option<u16>,
    pub descriptor: u64,
    pub module_handle: Option<u64>,
    pub bound_slots: Option<u64>,
    pub unload_slots: Option<u64>,
}
#[derive(Clone, Copy, Debug)]
pub struct Tls {
    pub address: u64,
    pub file_bytes: usize,
    pub zero_bytes: usize,
    pub index: Option<u64>,
    pub alignment: usize,
}
#[derive(Clone, Copy, Debug)]
pub struct Relocation {
    pub address: u64,
    pub kind: u32,
    pub symbol: Option<usize>,
    pub addend: Option<i64>,
}
pub struct Image {
    pub target: Target,
    pub preferred_base: u64,
    pub base: u64,
    pub entry: u64,
    pub bytes: Box<[u8]>,
    pub regions: Box<[Region]>,
    pub names: NameTable,
    pub symbols: Box<[Symbol]>,
    pub imports: Box<[Import]>,
    pub relocations: Box<[Relocation]>,
    pub needed: Box<[NameId]>,
    pub tls: Option<Tls>,
    pub initializers: Box<[u64]>,
    pub dynamic: Box<[(u64, u64)]>,
    pub relro: Box<[(u64, usize)]>,
}
impl Image {
    pub fn parse(bytes: &[u8], base: Option<u64>, role: LoadRole) -> Result<Self, FormatError> {
        if bytes.len() > 512 * 1024 * 1024 {
            return Err(FormatError::InvalidRange);
        }
        if bytes.starts_with(b"MZ") {
            pe::parse(bytes, base)
        } else if bytes.starts_with(b"\x7fELF") {
            elf::parse(bytes, base, role)
        } else {
            Err(FormatError::Unsupported)
        }
    }
    pub fn symbol(&self, name: &[u8]) -> Option<&Symbol> {
        self.symbol_version(name, None)
    }
    pub fn symbol_version(&self, name: &[u8], version: Option<&[u8]>) -> Option<&Symbol> {
        let id = self.names.find(name)?;
        let version = match version {
            Some(v) => Some(self.names.find(v)?),
            None => None,
        };
        self.symbols.iter().find(|s| {
            s.defined
                && s.name == Some(id)
                && matches!(s.binding, 1 | 2 | 10)
                && !matches!(s.visibility, 1 | 2)
                && match version {
                    Some(v) => s.version.is_some_and(|actual| actual.name == v),
                    None => !s.hidden_version,
                }
        })
    }
}
fn data(bytes: &[u8], offset: u64, length: usize) -> Result<&[u8], FormatError> {
    let at = usize::try_from(offset).map_err(|_| FormatError::InvalidRange)?;
    bytes
        .get(at..at.checked_add(length).ok_or(FormatError::InvalidRange)?)
        .ok_or(FormatError::InvalidRange)
}
fn mapped<'a>(
    bytes: &'a [u8],
    regions: &[Region],
    offset: u64,
    length: usize,
) -> Result<&'a [u8], FormatError> {
    let end = offset
        .checked_add(length as u64)
        .ok_or(FormatError::InvalidRange)?;
    let mut at = offset;
    for region in regions {
        let start = region.offset as u64;
        let limit = start + region.length as u64;
        if at >= start && at < limit {
            at = end.min(limit);
        }
    }
    if at != end {
        return Err(FormatError::InvalidRange);
    }
    data(bytes, offset, length)
}
fn word(bytes: &[u8], offset: u64, bits: u32) -> Result<u64, FormatError> {
    let mut r = Reader::new(data(bytes, offset, (bits / 8) as usize)?);
    match bits {
        16 => r.u16().map(u64::from),
        32 => r.u32().map(u64::from),
        64 => r.u64(),
        _ => Err(FormatError::Unsupported),
    }
}
fn terminated(bytes: &[u8], offset: u64) -> Result<&[u8], FormatError> {
    let at = usize::try_from(offset).map_err(|_| FormatError::InvalidRange)?;
    let tail = bytes.get(at..).ok_or(FormatError::InvalidRange)?;
    Ok(&tail[..tail
        .iter()
        .position(|&b| b == 0)
        .ok_or(FormatError::InvalidRange)?])
}
fn owned_length(value: u64) -> Result<usize, FormatError> {
    usize::try_from(value)
        .ok()
        .filter(|&n| n > 0 && n <= 512 * 1024 * 1024)
        .ok_or(FormatError::InvalidRange)
}
fn names(raw: &[&[u8]]) -> Result<NameTable, FormatError> {
    NameTable::load(raw.iter().copied()).map_err(|_| FormatError::InvalidRange)
}
fn name(table: &NameTable, text: &[u8]) -> Result<NameId, FormatError> {
    table.find(text).ok_or(FormatError::InvalidValue)
}
