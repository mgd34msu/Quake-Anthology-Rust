//! ELF image inspection: headers, dynamic tags, symbols, relocations.
//!
//! Donor: `src/guest/elf/parse.ts`. Program-header virtual addresses, never
//! section offsets, locate dynamic data. Only little-endian ELF version 1
//! for i386/x86-64 is supported.

use std::collections::HashMap;

use crate::core::contracts::NativeAbi;
use crate::error::GuestError;

fn elf_error(detail: impl Into<String>) -> GuestError {
    GuestError::bad_image("elf", detail)
}

/// ELF program segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElfSegment {
    /// Segment type.
    pub segment_type: u32,
    /// Segment flags.
    pub flags: u32,
    /// File offset.
    pub offset: usize,
    /// Virtual address.
    pub address: u64,
    /// File size.
    pub file_size: usize,
    /// Memory size.
    pub memory_size: usize,
    /// Alignment.
    pub alignment: u64,
}

/// ELF section header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElfSection {
    /// Section name.
    pub name: String,
    /// Section type.
    pub section_type: u32,
    /// Section flags.
    pub flags: u64,
    /// Virtual address.
    pub address: u64,
    /// File offset.
    pub offset: usize,
    /// Size in bytes.
    pub size: usize,
    /// Link index.
    pub link: u32,
    /// Info field.
    pub info: u32,
    /// Entry size.
    pub entry_size: usize,
}

/// ELF symbol version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElfVersion {
    /// Version name.
    pub name: String,
    /// Defining library.
    pub library: String,
    /// Hidden version.
    pub hidden: bool,
    /// Weak version.
    pub weak: bool,
}

/// ELF symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElfSymbol {
    /// Symbol index.
    pub index: usize,
    /// Symbol name.
    pub name: String,
    /// Symbol value.
    pub value: u64,
    /// Symbol size.
    pub size: usize,
    /// Symbol binding.
    pub binding: u8,
    /// Symbol type.
    pub symbol_type: u8,
    /// Symbol visibility.
    pub visibility: u8,
    /// Section index.
    pub section: u16,
    /// Symbol version, if any.
    pub version: Option<ElfVersion>,
}

/// Relocation table kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElfRelocationTable {
    /// REL table.
    Rel,
    /// RELA table.
    Rela,
    /// PLT REL table.
    PltRel,
    /// PLT RELA table.
    PltRela,
    /// RELR table.
    Relr,
}

/// ELF relocation. `None` addend means it lives in the mapped slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElfRelocation {
    /// Relocation address.
    pub address: u64,
    /// Relocation type.
    pub relocation_type: u32,
    /// Symbol index.
    pub symbol_index: usize,
    /// Explicit addend, if any.
    pub addend: Option<i64>,
    /// Source table.
    pub table: ElfRelocationTable,
}

/// Image type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElfType {
    /// Shared object.
    Shared,
    /// Executable.
    Executable,
}

/// Complete ELF inspection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElfInspection {
    /// Image ABI.
    pub abi: NativeAbi,
    /// Image type.
    pub image_type: ElfType,
    /// Entry point.
    pub entry_point: u64,
    /// Program segments.
    pub segments: Vec<ElfSegment>,
    /// Sections.
    pub sections: Vec<ElfSection>,
    /// Dynamic tags to values.
    pub dynamic: HashMap<u32, Vec<u64>>,
    /// Needed libraries.
    pub needed_libraries: Vec<String>,
    /// SONAME, if any.
    pub soname: Option<String>,
    /// Interpreter, if any.
    pub interpreter: Option<String>,
    /// RUNPATH, if any.
    pub runpath: Option<String>,
    /// RPATH, if any.
    pub rpath: Option<String>,
    /// Symbols.
    pub symbols: Vec<ElfSymbol>,
    /// Relocations.
    pub relocations: Vec<ElfRelocation>,
}

/// Checked host index from a file value.
pub fn checked_number(value: u64, description: &str) -> Result<usize, GuestError> {
    if value > usize::MAX as u64 {
        return Err(elf_error(format!("{description} exceeds checked host indexing")));
    }
    Ok(value as usize)
}

/// First value of a dynamic tag, if present.
#[must_use]
pub fn dynamic_value(dynamic: &HashMap<u32, Vec<u64>>, tag: u32) -> Option<u64> {
    dynamic.get(&tag)?.first().copied()
}

struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    fn range(&self, offset: usize, size: usize) -> Result<(), GuestError> {
        if offset > self.bytes.len().saturating_sub(size) {
            return Err(elf_error(format!(
                "file range {offset}+{size} exceeds {} bytes",
                self.bytes.len()
            )));
        }
        Ok(())
    }

    fn u8(&self, offset: usize) -> Result<u8, GuestError> {
        self.range(offset, 1)?;
        Ok(self.bytes[offset])
    }

    fn u16(&self, offset: usize) -> Result<u16, GuestError> {
        self.range(offset, 2)?;
        Ok(u16::from_le_bytes([self.bytes[offset], self.bytes[offset + 1]]))
    }

    fn u32(&self, offset: usize) -> Result<u32, GuestError> {
        self.range(offset, 4)?;
        Ok(u32::from_le_bytes([
            self.bytes[offset],
            self.bytes[offset + 1],
            self.bytes[offset + 2],
            self.bytes[offset + 3],
        ]))
    }

    fn word(&self, offset: usize, wide: bool) -> Result<u64, GuestError> {
        if wide {
            self.range(offset, 8)?;
            let mut word = [0u8; 8];
            word.copy_from_slice(&self.bytes[offset..offset + 8]);
            Ok(u64::from_le_bytes(word))
        } else {
            Ok(u64::from(self.u32(offset)?))
        }
    }

    fn signed(&self, offset: usize, wide: bool) -> Result<i64, GuestError> {
        if wide {
            self.range(offset, 8)?;
            let mut word = [0u8; 8];
            word.copy_from_slice(&self.bytes[offset..offset + 8]);
            Ok(i64::from_le_bytes(word))
        } else {
            Ok(i64::from(self.u32(offset)? as i32))
        }
    }

    fn string(&self, offset: usize, maximum: usize) -> Result<String, GuestError> {
        self.range(offset, maximum)?;
        let mut end = offset;
        while end < offset + maximum && self.bytes[end] != 0 {
            end += 1;
        }
        if end == offset + maximum {
            return Err(elf_error(format!("unterminated string at {offset}")));
        }
        Ok(String::from_utf8_lossy(&self.bytes[offset..end]).into_owned())
    }
}

/// Locate a virtual range in PT_LOAD file backing.
pub fn elf_file_offset(segments: &[ElfSegment], address: u64, size: usize) -> Result<usize, GuestError> {
    for segment in segments {
        if segment.segment_type != 1 || address < segment.address {
            continue;
        }
        let displacement = address - segment.address;
        if displacement + size as u64 <= segment.file_size as u64 {
            return Ok(segment.offset + checked_number(displacement, "segment displacement")?);
        }
    }
    Err(elf_error(format!(
        "virtual file range 0x{address:x}+{size} has no PT_LOAD backing"
    )))
}

/// Inspect an ELF image.
pub fn inspect_elf(bytes: &[u8]) -> Result<ElfInspection, GuestError> {
    let r = Reader { bytes };
    if r.u32(0)? != 0x464c_457f {
        return Err(elf_error("invalid magic"));
    }
    let class = r.u8(4)?;
    if class != 1 && class != 2 {
        return Err(elf_error(format!("unsupported class {class}")));
    }
    let wide = class == 2;
    if r.u8(5)? != 1 || r.u8(6)? != 1 || r.u32(20)? != 1 {
        return Err(elf_error("only little-endian ELF version 1 is supported"));
    }
    if (r.u8(7)? != 0 && r.u8(7)? != 3) || r.u8(8)? != 0 {
        return Err(elf_error("unsupported OS ABI or ABI version"));
    }
    let image_type = r.u16(16)?;
    if image_type != 2 && image_type != 3 {
        return Err(elf_error(format!("expected ET_EXEC or ET_DYN, found {image_type}")));
    }
    let machine = r.u16(18)?;
    if machine != if wide { 62 } else { 3 } {
        return Err(elf_error(format!(
            "unsupported class/machine combination {class}/{machine}"
        )));
    }
    if r.u32(if wide { 48 } else { 36 })? != 0 {
        return Err(elf_error("unsupported machine flags"));
    }
    if r.u16(if wide { 52 } else { 40 })? != if wide { 64 } else { 52 } {
        return Err(elf_error("incorrect ELF header size"));
    }
    let phoff = checked_number(r.word(if wide { 32 } else { 28 }, wide)?, "program table offset")?;
    let shoff = checked_number(r.word(if wide { 40 } else { 32 }, wide)?, "section table offset")?;
    let phsize = r.u16(if wide { 54 } else { 42 })?;
    let shsize = r.u16(if wide { 58 } else { 46 })?;
    let mut phnum = u32::from(r.u16(if wide { 56 } else { 44 })?);
    let mut shnum = u32::from(r.u16(if wide { 60 } else { 48 })?);
    let mut names_index = u32::from(r.u16(if wide { 62 } else { 50 })?);
    if shoff != 0 {
        if shsize != if wide { 64 } else { 40 } {
            return Err(elf_error("incorrect section header size"));
        }
        r.range(shoff, usize::from(shsize))?;
        if shnum == 0 {
            shnum = r.word(shoff + if wide { 32 } else { 20 }, wide)? as u32;
            checked_number(u64::from(shnum), "extended section count")?;
        }
        if phnum == 0xffff {
            phnum = r.u32(shoff + if wide { 44 } else { 28 })?;
        }
        if names_index == 0xffff {
            names_index = r.u32(shoff + if wide { 40 } else { 24 })?;
        }
    } else if shnum != 0 || names_index != 0 || phnum == 0xffff {
        return Err(elf_error("missing extended section header"));
    }
    if phsize != if wide { 56 } else { 32 } || phnum == 0 {
        return Err(elf_error("missing or invalid program table"));
    }
    let (phnum, shnum) = (
        checked_number(u64::from(phnum), "program count")?,
        checked_number(u64::from(shnum), "section count")?,
    );
    r.range(phoff, phnum * usize::from(phsize))?;
    r.range(shoff, shnum * usize::from(shsize))?;
    let mut segments = Vec::with_capacity(phnum);
    for i in 0..phnum {
        let p = phoff + i * usize::from(phsize);
        let segment = ElfSegment {
            segment_type: r.u32(p)?,
            flags: r.u32(p + if wide { 4 } else { 24 })?,
            offset: checked_number(r.word(p + if wide { 8 } else { 4 }, wide)?, "segment offset")?,
            address: r.word(p + if wide { 16 } else { 8 }, wide)?,
            file_size: checked_number(r.word(p + if wide { 32 } else { 16 }, wide)?, "segment file size")?,
            memory_size: checked_number(r.word(p + if wide { 40 } else { 20 }, wide)?, "segment memory size")?,
            alignment: r.word(p + if wide { 48 } else { 28 }, wide)?,
        };
        if segment.segment_type != 0 {
            r.range(segment.offset, segment.file_size)?;
        }
        if segment.segment_type == 1 || segment.segment_type == 7 {
            if segment.file_size > segment.memory_size {
                return Err(elf_error("segment file size exceeds memory size"));
            }
            let alignment = segment.alignment;
            if alignment > 1
                && (alignment & (alignment - 1) != 0
                    || !segment.address.wrapping_sub(segment.offset as u64).is_multiple_of(alignment))
            {
                return Err(elf_error("invalid segment alignment or file/address congruence"));
            }
            if segment.address as u128 + segment.memory_size as u128 > (1u128 << if wide { 64 } else { 32 }) {
                return Err(elf_error("segment exceeds ELF address width"));
            }
        }
        segments.push(segment);
    }
    if !segments
        .iter()
        .any(|segment| segment.segment_type == 1 && segment.memory_size != 0)
    {
        return Err(elf_error("no loadable image"));
    }
    let mut raw_sections: Vec<(ElfSection, usize)> = Vec::with_capacity(shnum);
    for i in 0..shnum {
        let p = shoff + i * usize::from(shsize);
        let section = ElfSection {
            name: String::new(),
            section_type: r.u32(p + 4)?,
            flags: r.word(p + 8, wide)?,
            address: r.word(p + if wide { 16 } else { 12 }, wide)?,
            offset: checked_number(r.word(p + if wide { 24 } else { 16 }, wide)?, "section offset")?,
            size: checked_number(r.word(p + if wide { 32 } else { 20 }, wide)?, "section size")?,
            link: r.u32(p + if wide { 40 } else { 24 })?,
            info: r.u32(p + if wide { 44 } else { 28 })?,
            entry_size: checked_number(r.word(p + if wide { 56 } else { 36 }, wide)?, "section entry size")?,
        };
        let name_offset = checked_number(u64::from(r.u32(p)?), "section name")?;
        if section.section_type != 8 && section.section_type != 0 {
            r.range(section.offset, section.size)?;
        }
        raw_sections.push((section, name_offset));
    }
    let names: Option<(usize, usize)> = if names_index == 0 {
        None
    } else {
        let names = raw_sections.get(names_index as usize).map(|(section, _)| section);
        if names.is_none_or(|names| names.section_type != 3) {
            return Err(elf_error("invalid section name table"));
        }
        names.map(|names| (names.offset, names.size))
    };
    let mut sections = Vec::with_capacity(raw_sections.len());
    for (mut section, name_offset) in raw_sections {
        if let Some((names_offset, names_size)) = names {
            if name_offset >= names_size {
                return Err(elf_error("section name exceeds string table"));
            }
            section.name = r.string(names_offset + name_offset, names_size - name_offset)?;
        }
        sections.push(section);
    }
    let dynamic = read_dynamic(&r, &segments, wide)?;
    let strtab = dynamic_value(&dynamic, 5);
    let strsize = checked_number(dynamic_value(&dynamic, 10).unwrap_or(0), "dynamic string size")?;
    let string_at = |index: u64| -> Result<String, GuestError> {
        let offset = checked_number(index, "string index")?;
        let Some(strtab) = strtab else {
            return Err(elf_error("dynamic string exceeds DT_STRTAB/DT_STRSZ"));
        };
        if offset >= strsize {
            return Err(elf_error("dynamic string exceeds DT_STRTAB/DT_STRSZ"));
        }
        r.string(
            elf_file_offset(&segments, strtab + index, strsize - offset)?,
            strsize - offset,
        )
    };
    let relocations = read_relocations(&r, &segments, &dynamic, wide)?;
    let versions = read_versions(&r, &segments, &dynamic, &string_at)?;
    let symbols = read_symbols(&r, &segments, &sections, &dynamic, wide, &versions, &string_at)?;
    for relocation in &relocations {
        if relocation.symbol_index >= symbols.len() && relocation.symbol_index != 0 {
            return Err(elf_error("relocation symbol index exceeds symbol table"));
        }
    }
    let interpreters: Vec<&ElfSegment> = segments.iter().filter(|segment| segment.segment_type == 3).collect();
    if interpreters.len() > 1 || segments.iter().filter(|segment| segment.segment_type == 7).count() > 1 {
        return Err(elf_error("multiple interpreter or TLS segments"));
    }
    let needed_libraries: Vec<String> = dynamic
        .get(&1)
        .map(|values| {
            values
                .iter()
                .map(|value| string_at(*value))
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    let soname = dynamic_value(&dynamic, 14).map(string_at).transpose()?;
    let runpath = dynamic_value(&dynamic, 29).map(string_at).transpose()?;
    let rpath = dynamic_value(&dynamic, 15).map(string_at).transpose()?;
    let interpreter = match interpreters.first() {
        Some(interpreter) => Some(r.string(interpreter.offset, interpreter.file_size)?),
        None => None,
    };
    Ok(ElfInspection {
        abi: if wide {
            NativeAbi::LinuxX86_64
        } else {
            NativeAbi::LinuxI386
        },
        image_type: if image_type == 3 {
            ElfType::Shared
        } else {
            ElfType::Executable
        },
        entry_point: r.word(24, wide)?,
        segments,
        sections,
        needed_libraries,
        soname,
        runpath,
        rpath,
        interpreter,
        symbols,
        relocations,
        dynamic,
    })
}

fn read_dynamic(r: &Reader, segments: &[ElfSegment], wide: bool) -> Result<HashMap<u32, Vec<u64>>, GuestError> {
    let mut entries: HashMap<u32, Vec<u64>> = HashMap::new();
    let tables: Vec<&ElfSegment> = segments.iter().filter(|segment| segment.segment_type == 2).collect();
    if tables.len() > 1 {
        return Err(elf_error("multiple dynamic segments"));
    }
    for segment in tables {
        let stride = if wide { 16 } else { 8 };
        let mut terminated = false;
        let mut offset = 0;
        while offset + stride <= segment.file_size {
            let p = segment.offset + offset;
            let tag = checked_number(r.word(p, wide)?, "dynamic tag")?;
            if tag > u32::MAX as usize {
                return Err(elf_error("dynamic tag exceeds 32 bits"));
            }
            let tag = tag as u32;
            if tag == 0 {
                terminated = true;
                break;
            }
            let values = entries.entry(tag).or_default();
            if !values.is_empty() && tag != 1 {
                return Err(elf_error(format!("duplicate dynamic tag 0x{tag:x}")));
            }
            values.push(r.word(p + if wide { 8 } else { 4 }, wide)?);
            offset += stride;
        }
        if !terminated {
            return Err(elf_error("dynamic table lacks DT_NULL"));
        }
    }
    Ok(entries)
}

#[allow(clippy::too_many_lines)]
fn read_relocations(
    r: &Reader,
    segments: &[ElfSegment],
    dynamic: &HashMap<u32, Vec<u64>>,
    wide: bool,
) -> Result<Vec<ElfRelocation>, GuestError> {
    let mut relocations = Vec::new();
    let mut seen_tables = std::collections::HashSet::new();
    let mut read = |address: u64, size: u64, rela: bool, table: ElfRelocationTable| -> Result<(), GuestError> {
        let stride = (if wide { 8 } else { 4 }) * if rela { 3 } else { 2 };
        let length = checked_number(size, "relocation table size")?;
        if length % stride != 0 {
            return Err(elf_error("partial relocation entry"));
        }
        let key = (address, length, rela);
        if !seen_tables.insert(key) {
            return Ok(());
        }
        let start = elf_file_offset(segments, address, length)?;
        let mut offset = 0;
        while offset < length {
            let p = start + offset;
            let info = r.word(p + if wide { 8 } else { 4 }, wide)?;
            relocations.push(ElfRelocation {
                address: r.word(p, wide)?,
                relocation_type: (info & if wide { 0xffff_ffff } else { 0xff }) as u32,
                symbol_index: checked_number(info >> if wide { 32 } else { 8 }, "relocation symbol")?,
                addend: if rela {
                    Some(r.signed(p + if wide { 16 } else { 8 }, wide)?)
                } else {
                    None
                },
                table,
            });
            offset += stride;
        }
        Ok(())
    };
    for rela in [false, true] {
        let address = dynamic_value(dynamic, if rela { 7 } else { 17 });
        let size = dynamic_value(dynamic, if rela { 8 } else { 18 });
        let entry = dynamic_value(dynamic, if rela { 9 } else { 19 });
        if address.is_none() && size.is_none() {
            continue;
        }
        let (Some(address), Some(size)) = (address, size) else {
            return Err(elf_error("incomplete REL/RELA dynamic tags"));
        };
        if entry != Some((if wide { 8 } else { 4 } * if rela { 3 } else { 2 }) as u64) {
            return Err(elf_error("incomplete REL/RELA dynamic tags"));
        }
        read(
            address,
            size,
            rela,
            if rela {
                ElfRelocationTable::Rela
            } else {
                ElfRelocationTable::Rel
            },
        )?;
    }
    if let Some(plt) = dynamic_value(dynamic, 23) {
        let kind = dynamic_value(dynamic, 20);
        let size = dynamic_value(dynamic, 2);
        if (kind != Some(7) && kind != Some(17)) || size.is_none() {
            return Err(elf_error("incomplete PLT relocation tags"));
        }
        let rela = kind == Some(7);
        read(
            plt,
            size.unwrap_or(0),
            rela,
            if rela {
                ElfRelocationTable::PltRela
            } else {
                ElfRelocationTable::PltRel
            },
        )?;
    }
    if let Some(relr) = dynamic_value(dynamic, 36) {
        let width = if wide { 8 } else { 4 };
        let size = dynamic_value(dynamic, 35);
        if size.is_none() || !size.unwrap_or(1).is_multiple_of(width as u64) || dynamic_value(dynamic, 37) != Some(width as u64) {
            return Err(elf_error("invalid RELR table"));
        }
        let length = checked_number(size.unwrap_or(0), "RELR table size")?;
        let offset = elf_file_offset(segments, relr, length)?;
        let mut cursor: Option<u64> = None;
        let mut i = 0;
        while i < length {
            let entry = r.word(offset + i, wide)?;
            if entry & 1 == 0 {
                relocations.push(ElfRelocation {
                    address: entry,
                    relocation_type: 8,
                    symbol_index: 0,
                    addend: None,
                    table: ElfRelocationTable::Relr,
                });
                cursor = Some(entry + width as u64);
            } else {
                let Some(cursor_value) = cursor else {
                    return Err(elf_error("RELR bitmap precedes address"));
                };
                for bit in 1..width * 8 {
                    if entry & (1 << bit) != 0 {
                        relocations.push(ElfRelocation {
                            address: cursor_value + ((bit - 1) * width) as u64,
                            relocation_type: 8,
                            symbol_index: 0,
                            addend: None,
                            table: ElfRelocationTable::Relr,
                        });
                    }
                }
                cursor = Some(cursor_value + ((width * 8 - 1) * width) as u64);
            }
            i += width;
        }
    }
    Ok(relocations)
}

struct PlainVersion {
    name: String,
    library: String,
    weak: bool,
}

fn read_versions(
    r: &Reader,
    segments: &[ElfSegment],
    dynamic: &HashMap<u32, Vec<u64>>,
    string_at: &dyn Fn(u64) -> Result<String, GuestError>,
) -> Result<HashMap<u16, PlainVersion>, GuestError> {
    let mut versions: HashMap<u16, PlainVersion> = HashMap::new();
    for needed in [true, false] {
        let address = dynamic_value(dynamic, if needed { 0x6fff_fffe } else { 0x6fff_fffc });
        let count = checked_number(
            dynamic_value(dynamic, if needed { 0x6fff_ffff } else { 0x6fff_fffd }).unwrap_or(0),
            "version count",
        )?;
        if address.is_none() {
            if count != 0 {
                return Err(elf_error("missing version table"));
            }
            continue;
        }
        if count == 0 {
            return Err(elf_error("version table has no count"));
        }
        let mut current = address.unwrap_or(0);
        for i in 0..count {
            let p = elf_file_offset(segments, current, if needed { 16 } else { 20 })?;
            if r.u16(p)? != 1 {
                return Err(elf_error("unsupported symbol version record"));
            }
            let aux_count = r.u16(p + if needed { 2 } else { 6 })?;
            let mut auxiliary = current + u64::from(r.u32(p + if needed { 8 } else { 12 })?);
            if aux_count == 0 {
                return Err(elf_error("version record has no name"));
            }
            for j in 0..aux_count {
                let a = elf_file_offset(segments, auxiliary, if needed { 16 } else { 8 })?;
                if needed || j == 0 {
                    let index = (if needed { r.u16(a + 6)? } else { r.u16(p + 4)? }) & 0x7fff;
                    let flags = if needed { r.u16(a + 4)? } else { r.u16(p + 2)? };
                    let version = PlainVersion {
                        name: string_at(u64::from(r.u32(a + if needed { 8 } else { 0 })?))?,
                        library: if needed {
                            string_at(u64::from(r.u32(p + 4)?))?
                        } else {
                            String::new()
                        },
                        weak: flags & 2 != 0,
                    };
                    if versions.contains_key(&index) {
                        return Err(elf_error(format!("duplicate version index {index}")));
                    }
                    versions.insert(index, version);
                }
                let next = r.u32(a + if needed { 12 } else { 4 })?;
                if u32::from(j) + 1 < u32::from(aux_count) && next == 0 {
                    return Err(elf_error("truncated version auxiliary chain"));
                }
                auxiliary += u64::from(next);
            }
            let next = r.u32(p + if needed { 12 } else { 16 })?;
            if i + 1 < count && next == 0 {
                return Err(elf_error("truncated version chain"));
            }
            current += u64::from(next);
        }
    }
    Ok(versions)
}

#[allow(clippy::too_many_lines)]
fn read_symbols(
    r: &Reader,
    segments: &[ElfSegment],
    sections: &[ElfSection],
    dynamic: &HashMap<u32, Vec<u64>>,
    wide: bool,
    versions: &HashMap<u16, PlainVersion>,
    dynamic_string: &dyn Fn(u64) -> Result<String, GuestError>,
) -> Result<Vec<ElfSymbol>, GuestError> {
    let symtab = dynamic_value(dynamic, 6);
    let symbol_section = if symtab.is_none() {
        sections.iter().find(|section| section.section_type == 2)
    } else {
        sections
            .iter()
            .find(|section| section.section_type == 11 && Some(section.address) == symtab)
    };
    let stride = if wide { 24 } else { 16 };
    let (count, offset): (usize, usize);
    let mut section_string: Option<(usize, usize)> = None;
    if symtab.is_none() {
        let Some(symbol_section) = symbol_section else {
            return Ok(Vec::new());
        };
        let strings = sections.get(symbol_section.link as usize);
        if strings.is_none_or(|strings| strings.section_type != 3) {
            return Err(elf_error("symbol table lacks string section"));
        }
        let strings = strings.unwrap_or_else(|| unreachable!("checked above"));
        section_string = Some((strings.offset, strings.size));
        count = symbol_section.size / stride;
        offset = symbol_section.offset;
    } else {
        if dynamic_value(dynamic, 11) != Some(stride as u64) {
            return Err(elf_error("incorrect DT_SYMENT"));
        }
        let hash = dynamic_value(dynamic, 4);
        let gnu_hash = dynamic_value(dynamic, 0x6fff_fef5);
        if let Some(hash) = hash {
            count = r.u32(elf_file_offset(segments, hash, 8)? + 4)? as usize;
        } else if let Some(gnu_hash) = gnu_hash {
            count = gnu_symbol_count(r, segments, gnu_hash, wide)?;
        } else if let Some(symbol_section) = symbol_section {
            count = symbol_section.size / stride;
        } else {
            return Err(elf_error(
                "cannot bound dynamic symbols without hash or section metadata",
            ));
        }
        offset = elf_file_offset(segments, symtab.unwrap_or(0), count * stride)?;
    }
    if symbol_section.is_some_and(|section| section.entry_size != stride || section.size / stride != count) {
        return Err(elf_error("symbol count or entry size disagrees with section metadata"));
    }
    r.range(offset, count * stride)?;
    let versym = dynamic_value(dynamic, 0x6fff_fff0);
    let version_offset = match versym {
        Some(versym) => Some(elf_file_offset(segments, versym, count * 2)?),
        None => None,
    };
    let string_at = |index: u64| -> Result<String, GuestError> {
        match section_string {
            Some((offset, size)) => {
                let i = checked_number(index, "symbol name")?;
                if i >= size {
                    return Err(elf_error("symbol name exceeds string table"));
                }
                r.string(offset + i, size - i)
            }
            None => dynamic_string(index),
        }
    };
    let mut symbols = Vec::with_capacity(count);
    for index in 0..count {
        let p = offset + index * stride;
        let info = r.u8(p + if wide { 4 } else { 12 })?;
        let version_word = match version_offset {
            Some(version_offset) => r.u16(version_offset + index * 2)?,
            None => 1,
        };
        let version_index = version_word & 0x7fff;
        let version = if version_index > 1 {
            versions.get(&version_index)
        } else {
            None
        };
        if version_index > 1 && version.is_none() {
            return Err(elf_error(format!("undefined version index {version_index}")));
        }
        let section = r.u16(p + if wide { 6 } else { 14 })?;
        if section == 0xffff {
            return Err(elf_error("extended symbol section indices are unsupported"));
        }
        symbols.push(ElfSymbol {
            index,
            name: string_at(u64::from(r.u32(p)?))?,
            value: r.word(p + if wide { 8 } else { 4 }, wide)?,
            size: checked_number(r.word(p + if wide { 16 } else { 8 }, wide)?, "symbol size")?,
            binding: if version_index == 0 && section != 0 {
                0
            } else {
                info >> 4
            },
            symbol_type: info & 15,
            visibility: r.u8(p + if wide { 5 } else { 13 })? & 3,
            section,
            version: version.map(|version| ElfVersion {
                name: version.name.clone(),
                library: version.library.clone(),
                hidden: version_word & 0x8000 != 0,
                weak: version.weak,
            }),
        });
    }
    Ok(symbols)
}

fn gnu_symbol_count(r: &Reader, segments: &[ElfSegment], address: u64, wide: bool) -> Result<usize, GuestError> {
    let header = elf_file_offset(segments, address, 16)?;
    let buckets = r.u32(header)?;
    let first = r.u32(header + 4)?;
    let bloom = r.u32(header + 8)?;
    if buckets == 0 || bloom == 0 {
        return Err(elf_error("invalid GNU hash table"));
    }
    let bucket_address = address + 16 + u64::from(bloom) * u64::from(if wide { 8u32 } else { 4u32 });
    let bucket_offset = elf_file_offset(segments, bucket_address, buckets as usize * 4)?;
    let chains = bucket_address + u64::from(buckets) * 4;
    let mut count = first;
    for bucket in 0..buckets {
        let mut symbol = r.u32(bucket_offset + bucket as usize * 4)?;
        if symbol == 0 {
            continue;
        }
        if symbol < first {
            return Err(elf_error("GNU hash bucket precedes symbol offset"));
        }
        loop {
            let hash = r.u32(elf_file_offset(segments, chains + u64::from(symbol - first) * 4, 4)?)?;
            count = count.max(symbol + 1);
            symbol += 1;
            if hash & 1 != 0 {
                break;
            }
        }
    }
    checked_number(u64::from(count), "GNU symbol count")
}
