use super::*;
struct ImportRaw<'a> {
    library: &'a [u8],
    label: Option<&'a [u8]>,
    ordinal: Option<u16>,
    slot: u64,
    delayed: bool,
    hint: Option<u16>,
    descriptor: u64,
    module_handle: Option<u64>,
    bound_slots: Option<u64>,
    unload_slots: Option<u64>,
}
pub(super) fn parse(file: &[u8], requested_base: Option<u64>) -> Result<Image, FormatError> {
    let pe = word(file, 0x3c, 32)?;
    if pe < 64 || data(file, pe, 4)? != b"PE\0\0" {
        return Err(FormatError::InvalidValue);
    }
    let mut coff = Reader::new(data(file, pe + 4, 20)?);
    let machine = coff.u16()?;
    let count = usize::from(coff.u16()?);
    coff.take(12)?;
    let optional_length = usize::from(coff.u16()?);
    let characteristics = coff.u16()?;
    if count == 0 || count > 96 || characteristics & 2 == 0 {
        return Err(FormatError::InvalidRange);
    }
    let optional = data(file, pe + 24, optional_length)?;
    let magic = word(optional, 0, 16)?;
    let bits = match (machine, magic) {
        (0x14c, 0x10b) => 32,
        (0x8664, 0x20b) => 64,
        _ => return Err(FormatError::Unsupported),
    };
    let directories_at = if bits == 64 { 112u64 } else { 96 };
    let preferred_base = word(optional, if bits == 64 { 24 } else { 28 }, bits)?;
    let base = requested_base.unwrap_or(preferred_base);
    let alignment = word(optional, 32, 32)?;
    let file_alignment = word(optional, 36, 32)?;
    let length = owned_length(word(optional, 56, 32)?)?;
    let headers = word(optional, 60, 32)? as usize;
    let entry = word(optional, 16, 32)?;
    if !alignment.is_power_of_two()
        || !file_alignment.is_power_of_two()
        || file_alignment > 65536
        || alignment < file_alignment
        || (alignment < 4096 && alignment != file_alignment)
        || (alignment >= 4096 && file_alignment < 512)
        || headers == 0
        || headers > file.len()
        || headers > length
        || !(headers as u64).is_multiple_of(file_alignment)
        || !(length as u64).is_multiple_of(alignment)
        || preferred_base == 0
        || base == 0
        || !base.is_multiple_of(65536)
        || !preferred_base.is_multiple_of(65536)
        || preferred_base.checked_add(length as u64).is_none()
        || base.checked_add(length as u64).is_none()
        || (bits == 32 && preferred_base + length as u64 > 1 << 32)
        || (bits == 32 && base + length as u64 > 1 << 32)
    {
        return Err(FormatError::InvalidRange);
    }
    let directory_count = word(optional, directories_at - 4, 32)? as usize;
    if directory_count > 16 {
        return Err(FormatError::InvalidRange);
    }
    data(optional, directories_at, directory_count * 8)?;
    let mut directories = [(0u32, 0usize); 16];
    for (i, item) in directories.iter_mut().enumerate().take(directory_count) {
        *item = (
            word(optional, directories_at + i as u64 * 8, 32)? as u32,
            word(optional, directories_at + i as u64 * 8 + 4, 32)? as usize,
        );
        let limit = if i == 4 { file.len() } else { length };
        if ((item.0 == 0) != (item.1 == 0) && i != 8)
            || u64::from(item.0) + item.1 as u64 > limit as u64
        {
            return Err(FormatError::InvalidRange);
        }
    }
    if directories[14].0 != 0 {
        return Err(FormatError::InvalidReference("PE CLR", 14));
    }
    let mut bytes = vec![0; length].into_boxed_slice();
    bytes[..headers].copy_from_slice(&file[..headers]);
    let mut regions = vec![Region {
        offset: 0,
        length: headers,
        read: true,
        write: false,
        execute: false,
    }];
    let table = pe + 24 + optional_length as u64;
    if table + count as u64 * 40 > headers as u64 {
        return Err(FormatError::InvalidRange);
    }
    data(file, table, count * 40)?;
    for i in 0..count {
        let section = data(file, table + i as u64 * 40, 40)?;
        let virtual_bytes = word(section, 8, 32)?;
        let offset = word(section, 12, 32)?;
        let raw_bytes = word(section, 16, 32)?;
        let raw_offset = word(section, 20, 32)?;
        let flags = word(section, 36, 32)?;
        let extent = virtual_bytes.max(raw_bytes);
        if extent == 0 {
            continue;
        }
        let mapped = extent
            .checked_add(alignment - 1)
            .map(|v| v & !(alignment - 1))
            .ok_or(FormatError::InvalidRange)?;
        if !offset.is_multiple_of(alignment)
            || offset < headers as u64
            || offset
                .checked_add(mapped)
                .is_none_or(|end| end > length as u64)
            || regions.iter().any(|r| {
                offset < r.offset as u64 + r.length as u64 && (r.offset as u64) < offset + mapped
            })
        {
            return Err(FormatError::InvalidRange);
        }
        if raw_bytes > 0 {
            if raw_offset < headers as u64
                || !raw_offset.is_multiple_of(file_alignment)
                || !raw_bytes.is_multiple_of(file_alignment)
            {
                return Err(FormatError::InvalidRange);
            }
            let source = data(file, raw_offset, raw_bytes as usize)?;
            bytes[offset as usize..offset as usize + source.len()].copy_from_slice(source);
        }
        regions.push(Region {
            offset: offset as usize,
            length: mapped as usize,
            read: flags & 0xc0000000 != 0,
            write: flags & 0x80000000 != 0,
            execute: flags & 0x20000000 != 0,
        });
    }
    regions.sort_unstable_by_key(|r| r.offset);
    let range = |rva: u64, count: usize| -> Result<&[u8], FormatError> {
        mapped(&bytes, &regions, rva, count)
    };
    for (i, &(rva, len)) in directories.iter().enumerate() {
        if i != 4 && rva != 0 {
            range(u64::from(rva), len)?;
        }
    }
    let text = |rva: u64| -> Result<&[u8], FormatError> {
        let r = regions
            .iter()
            .find(|r| rva >= r.offset as u64 && rva < r.offset as u64 + r.length as u64)
            .ok_or(FormatError::InvalidRange)?;
        let tail = range(rva, r.offset + r.length - rva as usize)?;
        let text = terminated(tail, 0)?;
        if !text.is_ascii() {
            return Err(FormatError::InvalidValue);
        }
        Ok(text)
    };
    if entry != 0
        && !regions.iter().any(|r| {
            r.execute && entry >= r.offset as u64 && entry < r.offset as u64 + r.length as u64
        })
    {
        return Err(FormatError::InvalidRange);
    }
    let mut raw_names = Vec::new();
    let mut raw_exports = Vec::new();
    let (exports_at, exports_bytes) = directories[0];
    if exports_at != 0 {
        let d = range(u64::from(exports_at), 40)?;
        let ordinal_base = word(d, 16, 32)? as u32;
        let function_count = word(d, 20, 32)? as usize;
        let name_count = word(d, 24, 32)? as usize;
        let functions = word(d, 28, 32)?;
        let named = word(d, 32, 32)?;
        let ordinals = word(d, 36, 32)?;
        let f = range(
            functions,
            function_count
                .checked_mul(4)
                .ok_or(FormatError::InvalidRange)?,
        )?;
        let n = range(
            named,
            name_count.checked_mul(4).ok_or(FormatError::InvalidRange)?,
        )?;
        let o = range(
            ordinals,
            name_count.checked_mul(2).ok_or(FormatError::InvalidRange)?,
        )?;
        let mut by_ordinal = vec![None; function_count];
        for (i, target) in by_ordinal.iter_mut().enumerate() {
            let rva = word(f, i as u64 * 4, 32)?;
            if rva == 0 {
                continue;
            }
            let forward = if rva >= u64::from(exports_at)
                && rva < u64::from(exports_at) + exports_bytes as u64
            {
                let remaining = (u64::from(exports_at) + exports_bytes as u64 - rva) as usize;
                let label = terminated(range(rva, remaining)?, 0)?;
                if !label.is_ascii() || !label.contains(&b'.') || label.last() == Some(&b'.') {
                    return Err(FormatError::InvalidValue);
                }
                raw_names.push(label);
                Some(label)
            } else {
                range(rva, 1)?;
                None
            };
            *target = Some(raw_exports.len());
            raw_exports.push((
                None,
                rva,
                ordinal_base
                    .checked_add(i as u32)
                    .ok_or(FormatError::InvalidRange)?,
                forward,
            ));
        }
        for i in 0..name_count {
            let ordinal = word(o, i as u64 * 2, 16)? as usize;
            let index = by_ordinal
                .get(ordinal)
                .copied()
                .flatten()
                .ok_or(FormatError::InvalidRange)?;
            let label = text(word(n, i as u64 * 4, 32)?)?;
            if label.is_empty() || raw_exports.iter().any(|e| e.0 == Some(label)) {
                return Err(FormatError::InvalidValue);
            }
            raw_names.push(label);
            let (_, rva, ordinal, forward) = raw_exports[index];
            raw_exports.push((Some(label), rva, ordinal, forward));
        }
    }

    let mut raw_imports = Vec::new();
    for (directory, delayed, stride) in [(1, false, 20usize), (13, true, 32)] {
        let (rva, len) = directories[directory];
        if rva == 0 {
            continue;
        }
        let descriptors = range(u64::from(rva), len)?;
        let mut terminated_descriptors = false;
        for (descriptor_index, d) in descriptors.chunks_exact(stride).enumerate() {
            if d.iter().all(|&b| b == 0) {
                terminated_descriptors = true;
                break;
            }
            let attributes = if delayed { word(d, 0, 32)? } else { 1 };
            if attributes & !1 != 0 {
                return Err(FormatError::InvalidValue);
            }
            let absolute = attributes & 1 == 0;
            let convert = |value: u64| -> Result<u64, FormatError> {
                if value == 0 {
                    Ok(0)
                } else if absolute {
                    value
                        .checked_sub(preferred_base)
                        .ok_or(FormatError::InvalidRange)
                } else {
                    Ok(value)
                }
            };
            let library_at = convert(word(d, if delayed { 4 } else { 12 }, 32)?)?;
            let slot = convert(word(d, if delayed { 12 } else { 16 }, 32)?)?;
            let original = convert(word(d, if delayed { 16 } else { 0 }, 32)?)?;
            let handle = if delayed {
                convert(word(d, 8, 32)?)?
            } else {
                0
            };
            let bound = if delayed {
                convert(word(d, 20, 32)?)?
            } else {
                0
            };
            let unload = if delayed {
                convert(word(d, 24, 32)?)?
            } else {
                0
            };
            let width = u64::from(bits / 8);
            if library_at == 0
                || slot == 0
                || [slot, original, handle, bound, unload]
                    .iter()
                    .any(|value| !value.is_multiple_of(width))
                || (delayed && (handle == 0 || original == 0))
                || (!delayed && original == 0 && word(d, 4, 32)? != 0)
            {
                return Err(FormatError::InvalidRange);
            }
            if handle != 0 {
                range(handle, width as usize)?;
            }
            let library = text(library_at)?;
            if library.is_empty() {
                return Err(FormatError::InvalidValue);
            }
            raw_names.push(library);
            let lookup = if original == 0 { slot } else { original };
            let mut at = 0u64;
            loop {
                let ptr_bytes = width as usize;
                let item = word(range(lookup + at, ptr_bytes)?, 0, bits)?;
                range(slot + at, ptr_bytes)?;
                for table in [bound, unload] {
                    if table != 0 {
                        range(table + at, ptr_bytes)?;
                    }
                }
                if item == 0 {
                    break;
                }
                let ordinal_bit = 1u64 << (bits - 1);
                let (label, ordinal, hint) = if item & ordinal_bit != 0 {
                    if item & !(ordinal_bit | 65535) != 0 {
                        return Err(FormatError::InvalidValue);
                    }
                    (None, Some(item as u16), None)
                } else {
                    let address = convert(item)?;
                    let hint = word(range(address, 2)?, 0, 16)? as u16;
                    let label = text(address + 2)?;
                    if label.is_empty() {
                        return Err(FormatError::InvalidValue);
                    }
                    raw_names.push(label);
                    (Some(label), None, Some(hint))
                };
                let address = base + slot + at;
                if raw_imports.iter().any(|i: &ImportRaw| i.slot == address) {
                    return Err(FormatError::InvalidRange);
                }
                raw_imports.push(ImportRaw {
                    library,
                    label,
                    ordinal,
                    slot: address,
                    delayed,
                    hint,
                    descriptor: base + u64::from(rva) + (descriptor_index * stride) as u64,
                    module_handle: (handle != 0).then_some(base + handle),
                    bound_slots: (bound != 0).then_some(base + bound + at),
                    unload_slots: (unload != 0).then_some(base + unload + at),
                });
                at = at.checked_add(width).ok_or(FormatError::InvalidRange)?;
            }
        }

        if !terminated_descriptors {
            return Err(FormatError::InvalidRange);
        }
    }
    let mut relocations = Vec::new();
    let mut patches = Vec::new();
    let delta = base.wrapping_sub(preferred_base);
    let (rva, len) = directories[5];
    if delta != 0 && (characteristics & 1 != 0 || rva == 0) {
        return Err(FormatError::InvalidReference("PE fixed base", 5));
    }
    if rva != 0 {
        let records = range(u64::from(rva), len)?;
        let mut r = Reader::new(records);
        while r.at < records.len() {
            let page = u64::from(r.u32()?);
            let size = r.u32()? as usize;
            if !page.is_multiple_of(4096) || size < 8 || !size.is_multiple_of(2) {
                return Err(FormatError::InvalidRange);
            }
            let mut entries = Reader::new(r.take(size - 8)?);
            while entries.at < entries.bytes.len() {
                let entry = entries.u16()?;
                let kind = u32::from(entry >> 12);
                if kind == 0 {
                    continue;
                }
                let width = match (bits, kind) {
                    (64, 10) => 8,
                    (32, 3) => 4,
                    (32, 1 | 2 | 4) => 2,
                    _ => return Err(FormatError::Unsupported),
                };
                let address = page + u64::from(entry & 4095);
                let value = word(range(address, width)?, 0, width as u32 * 8)?;
                let low = if kind == 4 {
                    Some(i64::from(entries.u16()? as i16))
                } else {
                    None
                };
                let patched = match kind {
                    1 => value.wrapping_add(delta >> 16),
                    4 => {
                        (value
                            .wrapping_shl(16)
                            .wrapping_add(low.ok_or(FormatError::InvalidValue)? as u64)
                            .wrapping_add(delta)
                            .wrapping_add(0x8000))
                            >> 16
                    }
                    _ => value.wrapping_add(delta),
                };
                patches.push((address as usize, width, patched));
                relocations.push(Relocation {
                    address: base + address,
                    bytes: width,
                    kind,
                    symbol: None,
                    addend: low,
                });
            }
        }
    }
    patches.sort_unstable_by_key(|p| p.0);
    if patches.windows(2).any(|p| p[0].0 + p[0].1 > p[1].0) {
        return Err(FormatError::InvalidRange);
    }
    let names = names(&raw_names)?;
    let symbols = raw_exports
        .into_iter()
        .map(|(label, rva, ordinal, forward)| {
            Ok(Symbol {
                name: label.map(|n| name(&names, n)).transpose()?,
                address: if rva == 0 { 0 } else { base + rva },
                bytes: 0,
                ordinal: Some(ordinal),
                forward: forward.map(|n| name(&names, n)).transpose()?,
                defined: rva != 0,
                absolute: false,
                weak: false,
                section: 0,
                binding: 1,
                kind: 0,
                visibility: 0,
                version: None,
                hidden_version: false,
            })
        })
        .collect::<Result<Box<[_]>, FormatError>>()?;
    let imports = raw_imports
        .into_iter()
        .map(|i| {
            Ok(Import {
                library: Some(name(&names, i.library)?),
                name: i.label.map(|n| name(&names, n)).transpose()?,
                ordinal: i.ordinal,
                slot: i.slot,
                delayed: i.delayed,
                hint: i.hint,
                descriptor: i.descriptor,
                module_handle: i.module_handle,
                bound_slots: i.bound_slots,
                unload_slots: i.unload_slots,
            })
        })
        .collect::<Result<Box<[_]>, FormatError>>()?;
    // TLS and initialization metadata are inert; executing them belongs to the
    // single native backend after imports and relocations have been resolved.
    let mut initializers = Vec::new();
    let mut tls = None;
    let (rva, len) = directories[9];
    if rva != 0 {
        let d = range(u64::from(rva), len)?;
        let width = u64::from(bits / 8);
        let start = word(d, 0, bits)?;
        let end = word(d, width, bits)?;
        let index = word(d, width * 2, bits)?;
        let callbacks = word(d, width * 3, bits)?;
        let zero = word(d, width * 4, 32)?;
        if end < start {
            return Err(FormatError::InvalidRange);
        }
        if end > start {
            range(
                start
                    .checked_sub(preferred_base)
                    .ok_or(FormatError::InvalidRange)?,
                (end - start) as usize,
            )?;
        }
        range(
            index
                .checked_sub(preferred_base)
                .ok_or(FormatError::InvalidRange)?,
            4,
        )?;
        let alignment = (word(d, width * 4 + 4, 32)? >> 20) & 15;
        if alignment == 15 {
            return Err(FormatError::InvalidValue);
        }
        tls = Some(Tls {
            address: if start == 0 {
                0
            } else {
                base + (start - preferred_base)
            },
            file_bytes: (end - start) as usize,
            zero_bytes: zero as usize,
            index: Some(base + (index - preferred_base)),
            alignment: if alignment == 0 {
                1
            } else {
                1usize << (alignment - 1)
            },
        });
        if callbacks != 0 {
            let mut at = callbacks
                .checked_sub(preferred_base)
                .ok_or(FormatError::InvalidRange)?;
            loop {
                let f = word(range(at, width as usize)?, 0, bits)?;
                if f == 0 {
                    break;
                }
                let offset = f
                    .checked_sub(preferred_base)
                    .ok_or(FormatError::InvalidRange)?;
                range(offset, 1)?;
                if !regions.iter().any(|r| {
                    r.execute
                        && offset >= r.offset as u64
                        && offset < r.offset as u64 + r.length as u64
                }) {
                    return Err(FormatError::InvalidRange);
                }
                initializers.push(base + offset);
                at += width;
            }
        }
    }
    for (offset, width, value) in patches {
        bytes[offset..offset + width].copy_from_slice(&value.to_le_bytes()[..width]);
    }
    Ok(Image {
        target: Target {
            bits,
            encoding: Encoding::Pe,
        },
        preferred_base,
        base,
        entry: if entry == 0 { 0 } else { base + entry },
        bytes,
        regions: regions.into_boxed_slice(),
        names,
        symbols,
        imports,
        relocations: relocations.into_boxed_slice(),
        needed: Box::new([]),
        tls,
        initializers: initializers.into_boxed_slice(),
        dynamic: Box::new([]),
        relro: Box::new([]),
    })
}
