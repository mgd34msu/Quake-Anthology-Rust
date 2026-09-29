//! PE data directories: imports, exports, TLS, unwind, load config.
//!
//! Donor: `src/guest/pe/directories.ts`. Delay-load imports are rejected;
//! every RVA resolves through the staged image reader.

use std::collections::{HashMap, HashSet};

use crate::core::contracts::{
    GuestAddress, GuestExport, GuestExportTarget, GuestImport, GuestSymbolName, GuestTlsTemplate,
    GuestUnwindFormat,
};
use crate::error::GuestError;
use crate::pe::format::{directory, pe_error, PeStage};
use crate::pe::image::{ImageReader, PeLoadConfiguration, PeUnwindRecord};

/// Read the import directory.
pub fn read_imports(image: &mut ImageReader) -> Result<Vec<GuestImport>, GuestError> {
    let reader = image.for_stage(PeStage::Imports);
    let mut imports = Vec::new();
    let mut slots = HashSet::new();
    let table = directory(&reader.pe, 1)?;
    if directory(&reader.pe, 13)?.rva != 0 {
        return Err(pe_error(
            PeStage::Imports,
            "delay-load imports require a guest delay-loader; unsupported",
        ));
    }
    if table.rva == 0 {
        return Ok(imports);
    }
    reader.range(table.rva, table.byte_length as usize)?;
    let width = reader.pe.abi.pointer_bytes() as u32;
    let ordinal_bit = 1u64 << (width * 8 - 1);
    let mut at = table.rva;
    while at + 20 <= table.rva + table.byte_length {
        let lookup = reader.u32(at)?;
        let timestamp = reader.u32(at + 4)?;
        let forward = reader.u32(at + 8)?;
        let name = reader.u32(at + 12)?;
        let iat = reader.u32(at + 16)?;
        if lookup == 0 && timestamp == 0 && forward == 0 && name == 0 && iat == 0 {
            return Ok(imports);
        }
        if name == 0
            || iat == 0
            || iat % width != 0
            || lookup % width != 0
            || (lookup == 0 && timestamp != 0)
        {
            return Err(pe_error(
                PeStage::Imports,
                "invalid descriptor/alignment or bound IAT without original lookup table",
            ));
        }
        let library = reader.text(name, reader.pe.image_size)?;
        if library.is_empty() {
            return Err(pe_error(PeStage::Imports, "empty import library"));
        }
        let mut index = 0u32;
        loop {
            let thunk = reader.pointer((if lookup == 0 { iat } else { lookup }) + index * width)?;
            let slot = iat + index * width;
            reader.range(slot, width as usize)?;
            if thunk == 0 {
                break;
            }
            let symbol = if thunk & ordinal_bit != 0 {
                if thunk & !(ordinal_bit | 0xffff) != 0 {
                    return Err(pe_error(PeStage::Imports, "ordinal thunk has reserved bits"));
                }
                GuestSymbolName::Ordinal((thunk & 0xffff) as u32)
            } else {
                if thunk > 0x7fff_ffff {
                    return Err(pe_error(PeStage::Imports, "name thunk is not a valid RVA"));
                }
                let rva = thunk as u32;
                reader.u16(rva)?;
                let name = reader.text(rva + 2, reader.pe.image_size)?;
                if name.is_empty() {
                    return Err(pe_error(PeStage::Imports, "empty import name"));
                }
                GuestSymbolName::Name { name, version: None }
            };
            if !slots.insert(slot) {
                return Err(pe_error(PeStage::Imports, "overlapping import address slots"));
            }
            let address = reader.address(slot, width as usize)?;
            imports.push(GuestImport {
                library: library.clone(),
                symbol,
                slot: address,
                weak: false,
            });
            index += 1;
        }
        at += 20;
    }
    Err(pe_error(PeStage::Imports, "unterminated import descriptor table"))
}

fn forwarded(text: &str) -> Result<GuestExportTarget, GuestError> {
    let separator = text.rfind('.');
    match separator {
        Some(separator) if separator > 0 && separator < text.len() - 1 => {
            let library = text[..separator].to_string();
            let name = &text[separator + 1..];
            if let Some(ordinal) = name.strip_prefix('#') {
                if ordinal.is_empty() || !ordinal.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(pe_error(PeStage::Exports, "invalid forwarded ordinal"));
                }
                let ordinal: u32 = ordinal
                    .parse()
                    .map_err(|_| pe_error(PeStage::Exports, "forwarded ordinal out of range"))?;
                if ordinal < 1 || ordinal > 65535 {
                    return Err(pe_error(PeStage::Exports, "forwarded ordinal out of range"));
                }
                return Ok(GuestExportTarget::Forward {
                    library,
                    symbol: GuestSymbolName::Ordinal(ordinal),
                });
            }
            Ok(GuestExportTarget::Forward {
                library,
                symbol: GuestSymbolName::Name {
                    name: name.to_string(),
                    version: None,
                },
            })
        }
        _ => Err(pe_error(
            PeStage::Exports,
            format!("invalid export forwarder {text}"),
        )),
    }
}

/// Read the export directory.
pub fn read_exports(image: &mut ImageReader) -> Result<Vec<GuestExport>, GuestError> {
    let reader = image.for_stage(PeStage::Exports);
    let table = directory(&reader.pe, 0)?;
    if table.rva == 0 {
        return Ok(Vec::new());
    }
    if table.byte_length < 40 {
        return Err(pe_error(PeStage::Exports, "truncated export directory"));
    }
    reader.range(table.rva, table.byte_length as usize)?;
    let ordinal_base = reader.u32(table.rva + 16)?;
    let count = reader.u32(table.rva + 20)?;
    let name_count = reader.u32(table.rva + 24)?;
    let addresses = reader.u32(table.rva + 28)?;
    let names = reader.u32(table.rva + 32)?;
    let ordinals = reader.u32(table.rva + 36)?;
    reader.range(addresses, count as usize * 4)?;
    reader.range(names, name_count as usize * 4)?;
    reader.range(ordinals, name_count as usize * 2)?;
    if ordinal_base as u64 + count as u64 > 0x1_0000_0000 {
        return Err(pe_error(PeStage::Exports, "export ordinal overflow"));
    }
    let mut targets: HashMap<u32, GuestExportTarget> = HashMap::new();
    let mut exports = Vec::new();
    for index in 0..count {
        let rva = reader.u32(addresses + index * 4)?;
        if rva == 0 {
            continue;
        }
        let target = if rva >= table.rva && rva < table.rva + table.byte_length {
            forwarded(&reader.text(rva, table.rva + table.byte_length)?)?
        } else {
            GuestExportTarget::Address(reader.address(rva, 1)?)
        };
        targets.insert(index, target.clone());
        exports.push(GuestExport {
            symbol: GuestSymbolName::Ordinal(ordinal_base + index),
            target,
        });
    }
    let mut seen = HashSet::new();
    for index in 0..name_count {
        let name = reader.text(reader.u32(names + index * 4)?, reader.pe.image_size)?;
        let target = targets.get(&(reader.u16(ordinals + index * 2)? as u32)).cloned();
        match (name.is_empty(), seen.insert(name.clone()), target) {
            (false, true, Some(target)) => exports.push(GuestExport {
                symbol: GuestSymbolName::Name { name, version: None },
                target,
            }),
            _ => {
                return Err(pe_error(
                    PeStage::Exports,
                    "duplicate name or invalid export ordinal target",
                ));
            }
        }
    }
    Ok(exports)
}

/// Read the TLS directory.
pub fn read_tls(
    image: &mut ImageReader,
) -> Result<(Option<GuestTlsTemplate>, Option<GuestAddress>), GuestError> {
    let reader = image.for_stage(PeStage::Tls);
    let table = directory(&reader.pe, 9)?;
    if table.rva == 0 {
        return Ok((None, None));
    }
    let width = reader.pe.abi.pointer_bytes() as u32;
    if table.byte_length < width * 4 + 8 {
        return Err(pe_error(PeStage::Tls, "truncated TLS directory"));
    }
    let start = reader.pointer(table.rva)?;
    let end = reader.pointer(table.rva + width)?;
    let index = reader.pointer(table.rva + width * 2)?;
    let callback_array = reader.pointer(table.rva + width * 3)?;
    let zero_fill_bytes = reader.u32(table.rva + width * 4)? as usize;
    let flags = reader.u32(table.rva + width * 4 + 4)?;
    let alignment_code = (flags >> 20) & 15;
    if end < start
        || end - start > u64::from(reader.pe.image_size)
        || alignment_code == 15
        || index == 0
    {
        return Err(pe_error(
            PeStage::Tls,
            "invalid TLS range, alignment or index address",
        ));
    }
    let size = (end - start) as usize;
    let initialized = if start == 0 && end == 0 {
        Vec::new()
    } else {
        let rva = reader.rva(start, size)?;
        reader.copy(rva, size)?
    };
    let mut callbacks = Vec::new();
    if callback_array != 0 {
        let mut at = reader.rva(callback_array, width as usize)?;
        loop {
            let callback = reader.pointer(at)?;
            if callback == 0 {
                break;
            }
            let rva = reader.rva(callback, 1)?;
            callbacks.push(reader.executable(rva)?);
            at += width;
        }
    }
    let module = reader.module.clone();
    let index_rva = reader.rva(index, 4)?;
    let index_address = reader.address(index_rva, 4)?;
    Ok((
        Some(GuestTlsTemplate {
            image: module,
            initialized,
            zero_fill_bytes,
            alignment: if alignment_code == 0 {
                1
            } else {
                1 << (alignment_code - 1)
            },
            callbacks,
        }),
        Some(index_address),
    ))
}

/// Read the exception (unwind) directory. x64 only.
pub fn read_unwind(image: &mut ImageReader) -> Result<Vec<PeUnwindRecord>, GuestError> {
    let reader = image.for_stage(PeStage::Unwind);
    let table = directory(&reader.pe, 3)?;
    if table.rva == 0 {
        return Ok(Vec::new());
    }
    if reader.pe.abi.pointer_bytes() != 8 || table.byte_length % 12 != 0 {
        return Err(pe_error(PeStage::Unwind, "unsupported exception directory layout"));
    }
    reader.range(table.rva, table.byte_length as usize)?;
    let mut result = Vec::new();
    let mut previous: i64 = -1;
    let mut at = table.rva;
    while at < table.rva + table.byte_length {
        let begin = reader.u32(at)?;
        if begin as i64 <= previous {
            return Err(pe_error(PeStage::Unwind, "unsorted runtime function table"));
        }
        previous = begin as i64;
        let record = read_unwind_entry(
            reader,
            begin,
            reader.u32(at + 4)?,
            reader.u32(at + 8)?,
            &HashSet::new(),
        )?;
        result.push(record);
        at += 12;
    }
    Ok(result)
}

fn read_unwind_entry(
    reader: &mut ImageReader,
    begin_rva: u32,
    end_rva: u32,
    unwind_info_rva: u32,
    ancestors: &HashSet<u32>,
) -> Result<PeUnwindRecord, GuestError> {
    if begin_rva >= end_rva || ancestors.contains(&unwind_info_rva) || ancestors.len() > 64 {
        return Err(pe_error(
            PeStage::Unwind,
            "invalid function range or cyclic unwind chain",
        ));
    }
    reader.executable(begin_rva)?;
    reader.executable(end_rva - 1)?;
    if unwind_info_rva % 4 != 0 {
        return Err(pe_error(PeStage::Unwind, "unaligned unwind metadata"));
    }
    let header = reader.u32(unwind_info_rva)?;
    let version = header & 7;
    let flags = (header >> 3) & 31;
    let count = (header >> 16) & 255;
    if (version != 1 && version != 2) || flags & !7 != 0 || (flags & 4 != 0 && flags & 3 != 0) {
        return Err(pe_error(
            PeStage::Unwind,
            format!("unsupported version/flags {version}/{flags}"),
        ));
    }
    let tail = unwind_info_rva + 4 + count.div_ceil(2) * 4;
    let mut chained = None;
    let mut handler_rva = None;
    let mut handler_data_rva = None;
    let mut size = (tail - unwind_info_rva) as usize;
    if flags & 4 != 0 {
        let mut next = ancestors.clone();
        next.insert(unwind_info_rva);
        chained = Some(Box::new(read_unwind_entry(
            reader,
            reader.u32(tail)?,
            reader.u32(tail + 4)?,
            reader.u32(tail + 8)?,
            &next,
        )?));
        size += 12;
    } else if flags & 3 != 0 {
        handler_rva = Some(reader.u32(tail)?);
        reader.executable(handler_rva.unwrap_or(0))?;
        handler_data_rva = Some(tail + 4);
        size += 4;
    }
    Ok(PeUnwindRecord {
        begin_rva,
        end_rva,
        unwind_info_rva,
        version,
        flags,
        handler_rva,
        handler_data_rva,
        chained,
        metadata: reader.copy(unwind_info_rva, size)?,
    })
}

/// Read the load-configuration directory.
pub fn read_load_configuration(
    image: &mut ImageReader,
) -> Result<Option<PeLoadConfiguration>, GuestError> {
    let reader = image.for_stage(PeStage::LoadConfig);
    let table = directory(&reader.pe, 10)?;
    if table.rva == 0 {
        return Ok(None);
    }
    let size = reader.u32(table.rva)? as usize;
    if size < 4 || size > table.byte_length as usize {
        return Err(pe_error(PeStage::LoadConfig, "invalid load configuration size"));
    }
    let width = reader.pe.abi.pointer_bytes() as u32;
    let guard_offset = if width == 4 { 88 } else { 144 };
    let address = reader.address(table.rva, size)?;
    let bytes = reader.copy(table.rva, size)?;
    let field = |reader: &mut ImageReader, offset: u32| -> Result<Option<GuestAddress>, GuestError> {
        if offset + width > size as u32 {
            return Ok(None);
        }
        let value = reader.pointer(table.rva + offset)?;
        if value == 0 {
            return Ok(None);
        }
        let rva = reader.rva(value, width as usize)?;
        Ok(Some(reader.address(rva, width as usize)?))
    };
    let security_cookie_address = field(reader, if width == 4 { 60 } else { 88 })?;
    let guard_check_slot = field(reader, if width == 4 { 72 } else { 112 })?;
    let guard_dispatch_slot = field(reader, if width == 4 { 76 } else { 120 })?;
    let guard_flags = if guard_offset + 4 <= size as u32 {
        reader.u32(table.rva + guard_offset)?
    } else {
        0
    };
    Ok(Some(PeLoadConfiguration {
        address,
        bytes,
        security_cookie_address,
        guard_check_slot,
        guard_dispatch_slot,
        guard_flags,
    }))
}

/// Unwind format for PE x64 records.
#[must_use]
pub const fn pe_unwind_format() -> GuestUnwindFormat {
    GuestUnwindFormat::PeX64Unwind
}
