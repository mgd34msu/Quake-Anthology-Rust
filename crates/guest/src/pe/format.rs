//! PE/COFF header parsing: sections, directories, images.
//!
//! Donor: `src/guest/pe/format.ts`. All RVAs resolve through the section
//! table; directory 4 (security) is file-based and never maps into the image.

use crate::core::contracts::{GuestPermissions, NativeAbi};
use crate::error::GuestError;

/// PE loader stage for error attribution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeStage {
    /// Header parsing.
    Headers,
    /// Image mapping.
    Mapping,
    /// Base relocation.
    Relocation,
    /// Import directory.
    Imports,
    /// Export directory.
    Exports,
    /// TLS directory.
    Tls,
    /// Unwind directory.
    Unwind,
    /// Load configuration.
    LoadConfig,
}

impl PeStage {
    /// Stage label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Headers => "headers",
            Self::Mapping => "mapping",
            Self::Relocation => "relocation",
            Self::Imports => "imports",
            Self::Exports => "exports",
            Self::Tls => "tls",
            Self::Unwind => "unwind",
            Self::LoadConfig => "load-config",
        }
    }
}

pub(crate) fn pe_error(stage: PeStage, detail: impl Into<String>) -> GuestError {
    GuestError::bad_image("pe", format!("PE {}: {}", stage.label(), detail.into()))
}

/// One data-directory entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeDirectory {
    /// Virtual address.
    pub rva: u32,
    /// Length in bytes.
    pub byte_length: u32,
}

/// PE section header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeSection {
    /// Section name.
    pub name: String,
    /// Virtual address.
    pub rva: u32,
    /// Virtual size.
    pub virtual_size: u32,
    /// Mapped size (aligned).
    pub mapped_size: u32,
    /// Raw data file offset.
    pub raw_offset: u32,
    /// Raw data size.
    pub raw_size: u32,
    /// Characteristics.
    pub characteristics: u32,
    /// Mapped permissions.
    pub permissions: GuestPermissions,
}

/// Parsed PE headers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeFile {
    /// Image ABI.
    pub abi: NativeAbi,
    /// Preferred image base.
    pub preferred_base: u64,
    /// Entry-point RVA.
    pub entry_point_rva: u32,
    /// Image size in bytes.
    pub image_size: u32,
    /// Headers size in bytes.
    pub header_size: u32,
    /// Section alignment.
    pub section_alignment: u32,
    /// File alignment.
    pub file_alignment: u32,
    /// File characteristics.
    pub characteristics: u16,
    /// DLL characteristics.
    pub dll_characteristics: u16,
    /// Sections.
    pub sections: Vec<PeSection>,
    /// Data directories (always 16 entries).
    pub directories: Vec<PeDirectory>,
}

/// Checked file range.
pub fn checked_range(
    offset: usize,
    length: usize,
    limit: usize,
    stage: PeStage,
) -> Result<(), GuestError> {
    if offset > limit || length > limit - offset {
        return Err(pe_error(
            stage,
            format!("range {offset}+{length} exceeds {limit}"),
        ));
    }
    Ok(())
}

/// Checked little-endian file reader.
#[derive(Debug, Clone)]
pub struct PeReader<'a> {
    /// File bytes.
    pub bytes: &'a [u8],
    /// Error stage.
    pub stage: PeStage,
}

impl<'a> PeReader<'a> {
    /// Reader over `bytes` attributing errors to `stage`.
    #[must_use]
    pub const fn new(bytes: &'a [u8], stage: PeStage) -> Self {
        Self { bytes, stage }
    }

    /// Checked `u16` load.
    pub fn u16(&self, offset: usize) -> Result<u16, GuestError> {
        checked_range(offset, 2, self.bytes.len(), self.stage)?;
        Ok(u16::from_le_bytes([self.bytes[offset], self.bytes[offset + 1]]))
    }

    /// Checked `u32` load.
    pub fn u32(&self, offset: usize) -> Result<u32, GuestError> {
        checked_range(offset, 4, self.bytes.len(), self.stage)?;
        Ok(u32::from_le_bytes([
            self.bytes[offset],
            self.bytes[offset + 1],
            self.bytes[offset + 2],
            self.bytes[offset + 3],
        ]))
    }

    /// Checked `u64` load.
    pub fn u64(&self, offset: usize) -> Result<u64, GuestError> {
        checked_range(offset, 8, self.bytes.len(), self.stage)?;
        let mut word = [0u8; 8];
        word.copy_from_slice(&self.bytes[offset..offset + 8]);
        Ok(u64::from_le_bytes(word))
    }

    /// Checked ASCII string within `limit` bytes.
    pub fn text(&self, offset: usize, limit: usize) -> Result<String, GuestError> {
        checked_range(offset, limit, self.bytes.len(), self.stage)?;
        let mut text = String::new();
        for byte in self.bytes.iter().skip(offset).take(limit) {
            if *byte == 0 {
                return Ok(text);
            }
            if *byte > 127 {
                return Err(pe_error(self.stage, "invalid ASCII string"));
            }
            text.push(*byte as char);
        }
        Err(pe_error(self.stage, "unterminated ASCII string"))
    }
}

fn power_of_two(value: u32) -> bool {
    value > 0 && value & (value - 1) == 0
}

fn align(value: u32, alignment: u32) -> u32 {
    value.next_multiple_of(alignment)
}

fn permissions(flags: u32) -> GuestPermissions {
    let read = flags & 0x4000_0000 != 0;
    let write = flags & 0x8000_0000 != 0;
    let execute = flags & 0x2000_0000 != 0;
    if write {
        if execute {
            GuestPermissions::ReadWriteExecute
        } else {
            GuestPermissions::ReadWrite
        }
    } else if execute {
        if read {
            GuestPermissions::ReadExecute
        } else {
            GuestPermissions::Execute
        }
    } else if read {
        GuestPermissions::Read
    } else {
        GuestPermissions::None
    }
}

/// Parse PE/COFF headers.
pub fn parse_pe(bytes: &[u8]) -> Result<PeFile, GuestError> {
    let reader = PeReader::new(bytes, PeStage::Headers);
    if reader.u16(0)? != 0x5a4d {
        return Err(pe_error(PeStage::Headers, "missing MZ signature"));
    }
    let pe = reader.u32(0x3c)? as usize;
    if pe < 64 || reader.u32(pe)? != 0x4550 {
        return Err(pe_error(PeStage::Headers, "invalid PE signature offset"));
    }
    let machine = reader.u16(pe + 4)?;
    let section_count = reader.u16(pe + 6)? as usize;
    let optional_size = reader.u16(pe + 20)? as usize;
    let characteristics = reader.u16(pe + 22)?;
    if section_count < 1 || section_count > 96 || characteristics & 2 == 0 {
        return Err(pe_error(
            PeStage::Headers,
            "invalid executable section count/characteristics",
        ));
    }
    let optional = pe + 24;
    checked_range(optional, optional_size, bytes.len(), PeStage::Headers)?;
    let magic = reader.u16(optional)?;
    let (abi, directory_offset) = match (machine, magic) {
        (0x14c, 0x10b) => (NativeAbi::WindowsI386, 96),
        (0x8664, 0x20b) => (NativeAbi::WindowsX86_64, 112),
        _ => {
            return Err(pe_error(
                PeStage::Headers,
                format!("unsupported machine/magic 0x{machine:x}/0x{magic:x}"),
            ));
        }
    };
    if optional_size < directory_offset {
        return Err(pe_error(PeStage::Headers, "truncated optional header"));
    }
    let preferred_base = if abi.pointer_bytes() == 4 {
        u64::from(reader.u32(optional + 28)?)
    } else {
        reader.u64(optional + 24)?
    };
    let entry_point_rva = reader.u32(optional + 16)?;
    let section_alignment = reader.u32(optional + 32)?;
    let file_alignment = reader.u32(optional + 36)?;
    let image_size = reader.u32(optional + 56)?;
    let header_size = reader.u32(optional + 60)?;
    if !power_of_two(section_alignment)
        || !power_of_two(file_alignment)
        || file_alignment > 65536
        || section_alignment < file_alignment
        || (if section_alignment < 4096 {
            file_alignment != section_alignment
        } else {
            file_alignment < 512
        })
    {
        return Err(pe_error(PeStage::Headers, "invalid section/file alignment"));
    }
    if preferred_base == 0
        || preferred_base % 65536 != 0
        || image_size == 0
        || image_size % section_alignment != 0
        || header_size == 0
        || header_size % file_alignment != 0
        || header_size > image_size
        || preferred_base as u128 + u64::from(image_size) as u128 > 1u128 << (abi.pointer_bytes() * 8)
    {
        return Err(pe_error(
            PeStage::Headers,
            "invalid image base/size or header size",
        ));
    }
    checked_range(0, header_size as usize, bytes.len(), PeStage::Headers)?;
    let directory_count = reader.u32(optional + directory_offset - 4)? as usize;
    if directory_count > 16 || directory_offset + directory_count * 8 > optional_size {
        return Err(pe_error(
            PeStage::Headers,
            "unsupported/truncated data directory array",
        ));
    }
    let mut directories = Vec::with_capacity(16);
    for i in 0..16 {
        let (rva, byte_length) = if i < directory_count {
            (
                reader.u32(optional + directory_offset + i * 8)?,
                reader.u32(optional + directory_offset + i * 8 + 4)?,
            )
        } else {
            (0, 0)
        };
        if (rva == 0) != (byte_length == 0) && i != 8 {
            return Err(pe_error(
                PeStage::Headers,
                format!("inconsistent data directory {i}"),
            ));
        }
        checked_range(
            rva as usize,
            byte_length as usize,
            if i == 4 { bytes.len() } else { image_size as usize },
            PeStage::Headers,
        )?;
        directories.push(PeDirectory { rva, byte_length });
    }
    let mut sections = Vec::with_capacity(section_count);
    let table = optional + optional_size;
    checked_range(table, section_count * 40, header_size as usize, PeStage::Headers)?;
    let mut end = align(header_size, section_alignment);
    for i in 0..section_count {
        let at = table + i * 40;
        let name_bytes = &bytes[at..at + 8];
        let nul = name_bytes.iter().position(|byte| *byte == 0).unwrap_or(8);
        let name = std::str::from_utf8(&name_bytes[..nul])
            .map_err(|_| pe_error(PeStage::Headers, "invalid section name"))?
            .to_string();
        let virtual_size = reader.u32(at + 8)?;
        let rva = reader.u32(at + 12)?;
        let raw_size = reader.u32(at + 16)?;
        let raw_offset = reader.u32(at + 20)?;
        let flags = reader.u32(at + 36)?;
        let mapped_size = align(virtual_size.max(raw_size), section_alignment);
        if rva % section_alignment != 0
            || rva < end
            || (raw_size > 0
                && (raw_offset < header_size
                    || raw_offset % file_alignment != 0
                    || raw_size % file_alignment != 0))
        {
            return Err(pe_error(
                PeStage::Headers,
                format!("overlapping/misaligned section {name}"),
            ));
        }
        checked_range(raw_offset as usize, raw_size as usize, bytes.len(), PeStage::Headers)?;
        checked_range(rva as usize, mapped_size as usize, image_size as usize, PeStage::Headers)?;
        end = rva + mapped_size;
        sections.push(PeSection {
            name,
            rva,
            virtual_size,
            mapped_size,
            raw_offset,
            raw_size,
            characteristics: flags,
            permissions: permissions(flags),
        });
    }
    Ok(PeFile {
        abi,
        preferred_base,
        entry_point_rva,
        image_size,
        header_size,
        section_alignment,
        file_alignment,
        characteristics,
        dll_characteristics: reader.u16(optional + 70)?,
        sections,
        directories,
    })
}

/// Fetch data directory `index`.
pub fn directory(file: &PeFile, index: usize) -> Result<PeDirectory, GuestError> {
    file.directories.get(index).copied().ok_or_else(|| {
        pe_error(PeStage::Headers, format!("invalid directory index {index}"))
    })
}
