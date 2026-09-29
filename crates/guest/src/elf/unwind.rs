//! ELF unwind metadata: `.eh_frame`/`.debug_frame` and GNU EH headers.
//!
//! Donor: `src/guest/elf/unwind.ts`. Preserves full CIE/FDE bytes while
//! indexing each function's actual covered PC range.

use crate::core::contracts::{GuestUnwindFormat, GuestUnwindRegion};
use crate::core::memory::SparseGuestMemory;
use crate::elf::parse::{checked_number, elf_file_offset, ElfInspection};
use crate::elf::relocate::elf_address;
use crate::error::GuestError;

fn elf_error(detail: impl Into<String>) -> GuestError {
    GuestError::bad_image("elf", detail)
}

struct Cursor {
    position: usize,
}

struct Cie {
    encoding: u8,
    augmentation: bool,
}

struct DwarfReader<'a> {
    bytes: &'a [u8],
    address: u64,
    pointer_bytes: usize,
    data_relative_base: Option<u64>,
}

impl<'a> DwarfReader<'a> {
    fn require(&self, cursor: &mut Cursor, bytes: usize, end: usize) -> Result<usize, GuestError> {
        let offset = cursor.position;
        if offset > end.saturating_sub(bytes) {
            return Err(elf_error("truncated DWARF unwind record"));
        }
        cursor.position += bytes;
        Ok(offset)
    }

    fn byte(&self, cursor: &mut Cursor) -> Result<u8, GuestError> {
        let offset = self.require(cursor, 1, self.bytes.len())?;
        Ok(self.bytes[offset])
    }

    fn u32(&self, cursor: &mut Cursor) -> Result<u32, GuestError> {
        let offset = self.require(cursor, 4, self.bytes.len())?;
        Ok(u32::from_le_bytes([
            self.bytes[offset],
            self.bytes[offset + 1],
            self.bytes[offset + 2],
            self.bytes[offset + 3],
        ]))
    }

    fn u64(&self, cursor: &mut Cursor) -> Result<u64, GuestError> {
        let offset = self.require(cursor, 8, self.bytes.len())?;
        let mut word = [0u8; 8];
        word.copy_from_slice(&self.bytes[offset..offset + 8]);
        Ok(u64::from_le_bytes(word))
    }

    fn leb(&self, cursor: &mut Cursor, signed: bool) -> Result<i64, GuestError> {
        let mut value: u64 = 0;
        for shift in (0..70).step_by(7) {
            let byte = self.byte(cursor)?;
            value |= u64::from(byte & 127) << shift;
            if byte & 128 == 0 {
                return Ok(if signed && byte & 64 != 0 {
                    value.wrapping_sub(1 << (shift + 7)) as i64
                } else {
                    value as i64
                });
            }
        }
        Err(elf_error("oversized DWARF LEB128"))
    }

    fn encoded(&self, cursor: &mut Cursor, encoding: u8, relative: bool) -> Result<u64, GuestError> {
        if encoding == 0xff {
            return Err(elf_error("omitted DWARF pointer used as an address"));
        }
        if encoding & 0x70 == 0x50 {
            let absolute = self.address.wrapping_add(cursor.position as u64);
            let pointer = self.pointer_bytes as u64;
            cursor.position += ((pointer - absolute % pointer) % pointer) as usize;
        }
        let address = self.address.wrapping_add(cursor.position as u64);
        let mut value: i64 = match encoding & 15 {
            0 => {
                if self.pointer_bytes == 8 {
                    self.u64(cursor)? as i64
                } else {
                    i64::from(self.u32(cursor)?)
                }
            }
            1 => self.leb(cursor, false)?,
            2 => {
                let offset = self.require(cursor, 2, self.bytes.len())?;
                i64::from(u16::from_le_bytes([self.bytes[offset], self.bytes[offset + 1]]))
            }
            3 => i64::from(self.u32(cursor)?),
            4 => self.u64(cursor)? as i64,
            9 => self.leb(cursor, true)?,
            10 => {
                let offset = self.require(cursor, 2, self.bytes.len())?;
                i64::from(i16::from_le_bytes([self.bytes[offset], self.bytes[offset + 1]]))
            }
            11 => {
                let offset = self.require(cursor, 4, self.bytes.len())?;
                i64::from(i32::from_le_bytes([
                    self.bytes[offset],
                    self.bytes[offset + 1],
                    self.bytes[offset + 2],
                    self.bytes[offset + 3],
                ]))
            }
            12 => {
                let offset = self.require(cursor, 8, self.bytes.len())?;
                let mut word = [0u8; 8];
                word.copy_from_slice(&self.bytes[offset..offset + 8]);
                i64::from_le_bytes(word)
            }
            _ => {
                return Err(elf_error(format!("unsupported DWARF pointer encoding 0x{encoding:x}")));
            }
        };
        if relative {
            match encoding & 0x70 {
                0 | 0x50 => {}
                0x10 => value = value.wrapping_add(address as i64),
                0x30 => {
                    let Some(base) = self.data_relative_base else {
                        return Err(elf_error("DWARF data-relative pointer requires an explicit base"));
                    };
                    value = value.wrapping_add(base as i64);
                }
                _ => {
                    return Err(elf_error(format!(
                        "unsupported DWARF relative pointer base 0x{encoding:x}"
                    )));
                }
            }
        }
        Ok(value as u64)
    }
}

/// Read unwind regions from frame sections or the GNU EH header.
pub fn read_elf_unwind(
    elf: &ElfInspection,
    source: &[u8],
    memory: &mut SparseGuestMemory,
    load_bias: u64,
) -> Result<Vec<GuestUnwindRegion>, GuestError> {
    let mut regions = Vec::new();
    let sections: Vec<&crate::elf::parse::ElfSection> = elf
        .sections
        .iter()
        .filter(|section| section.name == ".eh_frame" || section.name == ".debug_frame")
        .collect();
    for section in &sections {
        let allocated = section.flags & 2 != 0;
        let bytes = if allocated {
            memory.copy(
                elf_address(memory, load_bias.wrapping_add(section.address))?,
                section.size,
            )?
        } else {
            source
                .get(section.offset..section.offset + section.size)
                .ok_or_else(|| elf_error("frame section exceeds source"))?
                .to_vec()
        };
        regions.extend(read_frames(
            &bytes,
            if allocated {
                load_bias.wrapping_add(section.address)
            } else {
                section.address
            },
            elf,
            memory,
            load_bias,
            section.name == ".debug_frame",
        )?);
    }
    if !sections.iter().any(|section| section.name == ".eh_frame") {
        if let Some(header) = elf.segments.iter().find(|segment| segment.segment_type == 0x6474_e550) {
            let bytes = memory.copy(
                elf_address(memory, load_bias.wrapping_add(header.address))?,
                header.file_size,
            )?;
            let address = load_bias.wrapping_add(header.address);
            let r = DwarfReader {
                bytes: &bytes,
                address,
                pointer_bytes: elf.abi.pointer_bytes(),
                data_relative_base: Some(address),
            };
            let mut cursor = Cursor { position: 0 };
            if r.byte(&mut cursor)? != 1 {
                return Err(elf_error("unsupported GNU EH frame header version"));
            }
            let encoding = r.byte(&mut cursor)?;
            r.byte(&mut cursor)?;
            r.byte(&mut cursor)?;
            if encoding & 0x80 != 0 {
                return Err(elf_error("indirect GNU EH frame section pointer is unsupported"));
            }
            let frame_address = r.encoded(&mut cursor, encoding, true)?;
            let original = frame_address.wrapping_sub(load_bias);
            let segment = elf.segments.iter().find(|item| {
                item.segment_type == 1 && original >= item.address && original < item.address + item.file_size as u64
            });
            let Some(segment) = segment else {
                return Err(elf_error("GNU EH frame pointer has no load segment"));
            };
            let available = segment.file_size - checked_number(original - segment.address, "EH frame displacement")?;
            elf_file_offset(&elf.segments, original, available)?;
            let bytes = memory.copy(elf_address(memory, frame_address)?, available)?;
            regions.extend(read_frames(&bytes, frame_address, elf, memory, load_bias, false)?);
        }
    }
    Ok(regions)
}

fn read_frames(
    bytes: &[u8],
    address: u64,
    elf: &ElfInspection,
    memory: &mut SparseGuestMemory,
    load_bias: u64,
    debug: bool,
) -> Result<Vec<GuestUnwindRegion>, GuestError> {
    let reader = DwarfReader {
        bytes,
        address,
        pointer_bytes: elf.abi.pointer_bytes(),
        data_relative_base: None,
    };
    let mut cursor = Cursor { position: 0 };
    let mut cies: std::collections::HashMap<usize, Cie> = std::collections::HashMap::new();
    let mut regions = Vec::new();
    while cursor.position < bytes.len() {
        let start = cursor.position;
        if bytes.len() - start < 4 && bytes[start..].iter().all(|value| *value == 0) {
            break;
        }
        let initial = reader.u32(&mut cursor)?;
        if initial == 0 {
            break;
        }
        let wide_length = initial == 0xffff_ffff;
        let length = if wide_length {
            checked_number(reader.u64(&mut cursor)?, "DWARF record length")?
        } else {
            initial as usize
        };
        let end = cursor.position + length;
        if end > bytes.len() || length < if wide_length { 8 } else { 4 } {
            return Err(elf_error("invalid DWARF record length"));
        }
        let id_position = cursor.position;
        let id = if wide_length {
            reader.u64(&mut cursor)?
        } else {
            u64::from(reader.u32(&mut cursor)?)
        };
        let is_cie = if debug {
            id == if wide_length { u64::MAX } else { u64::from(u32::MAX) }
        } else {
            id == 0
        };
        if is_cie {
            let version = reader.byte(&mut cursor)?;
            if version != 1 && version != 3 && version != 4 {
                return Err(elf_error(format!("unsupported CIE version {version}")));
            }
            let mut augmentation = String::new();
            loop {
                let c = reader.byte(&mut cursor)?;
                if c == 0 {
                    break;
                }
                augmentation.push(c as char);
            }
            if version == 4
                && (reader.byte(&mut cursor)? as usize != elf.abi.pointer_bytes() || reader.byte(&mut cursor)? != 0)
            {
                return Err(elf_error("unsupported CIE address or segment size"));
            }
            reader.leb(&mut cursor, false)?;
            reader.leb(&mut cursor, true)?;
            if version == 1 {
                reader.byte(&mut cursor)?;
            } else {
                reader.leb(&mut cursor, false)?;
            }
            let mut encoding = 0;
            let has_augmentation = augmentation.starts_with('z');
            if has_augmentation {
                let size = checked_number(reader.leb(&mut cursor, false)? as u64, "CIE augmentation size")?;
                let augmentation_end = cursor.position + size;
                if augmentation_end > end {
                    return Err(elf_error("CIE augmentation exceeds record"));
                }
                for code in augmentation.chars().skip(1) {
                    match code {
                        'R' => encoding = reader.byte(&mut cursor)?,
                        'L' => {
                            reader.byte(&mut cursor)?;
                        }
                        'P' => {
                            let personality = reader.byte(&mut cursor)?;
                            reader.encoded(&mut cursor, personality, false)?;
                        }
                        'S' => {}
                        _ => return Err(elf_error(format!("unsupported CIE augmentation {code}"))),
                    }
                }
                if cursor.position > augmentation_end {
                    return Err(elf_error("CIE fields exceed augmentation size"));
                }
                cursor.position = augmentation_end;
            } else if !augmentation.is_empty() {
                return Err(elf_error(format!("unsupported CIE augmentation {augmentation}")));
            }
            cies.insert(
                start,
                Cie {
                    encoding,
                    augmentation: has_augmentation,
                },
            );
        } else {
            let cie_position = if debug {
                checked_number(id, "debug CIE offset")?
            } else {
                id_position - checked_number(id, "CIE displacement")?
            };
            let Some(cie) = cies.get(&cie_position) else {
                return Err(elf_error("FDE references an unavailable CIE"));
            };
            let mut pc = reader.encoded(&mut cursor, cie.encoding, true)?;
            if cie.encoding & 0x80 != 0 {
                let slot = memory.copy(elf_address(memory, pc)?, elf.abi.pointer_bytes())?;
                pc = if elf.abi.pointer_bytes() == 8 {
                    let mut word = [0u8; 8];
                    word.copy_from_slice(&slot);
                    u64::from_le_bytes(word)
                } else {
                    u64::from(u32::from_le_bytes([slot[0], slot[1], slot[2], slot[3]]))
                };
            }
            let range = reader.encoded(&mut cursor, cie.encoding & 15, false)? as i64;
            if debug {
                pc = pc.wrapping_add(load_bias);
            }
            if cie.augmentation {
                let size = checked_number(reader.leb(&mut cursor, false)? as u64, "FDE augmentation size")?;
                reader.require(&mut cursor, size, end)?;
            }
            if range < 0 {
                return Err(elf_error("negative FDE PC range"));
            }
            if range != 0 {
                regions.push(GuestUnwindRegion {
                    start: elf_address(memory, pc)?,
                    end: elf_address(memory, pc.wrapping_add(range as u64))?,
                    format: if debug {
                        GuestUnwindFormat::ElfDebugFrame
                    } else {
                        GuestUnwindFormat::ElfEhFrame
                    },
                    metadata: bytes.to_vec(),
                });
            }
        }
        if cursor.position > end {
            return Err(elf_error("unwind fields exceed record"));
        }
        cursor.position = end;
    }
    Ok(regions)
}
