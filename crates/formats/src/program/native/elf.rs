use super::*;

mod metadata;
mod relocations;
#[derive(Clone, Copy)]
struct Segment {
    kind: u32,
    flags: u32,
    offset: u64,
    address: u64,
    file_bytes: u64,
    memory_bytes: u64,
    alignment: u64,
}
fn page_end(value: u64) -> Result<u64, FormatError> {
    value
        .checked_add(4095)
        .map(|v| v & !4095)
        .ok_or(FormatError::InvalidRange)
}
pub(super) fn parse(
    file: &[u8],
    requested: Option<u64>,
    role: LoadRole,
) -> Result<Image, FormatError> {
    let bits = match data(file, 4, 5)? {
        [1, 1, 1, 0 | 3, 0] => 32,
        [2, 1, 1, 0 | 3, 0] => 64,
        _ => return Err(FormatError::Unsupported),
    };
    let wide = bits == 64;
    data(file, 0, if wide { 64 } else { 52 })?;
    let kind = word(file, 16, 16)?;
    let machine = word(file, 18, 16)?;
    if !matches!(kind, 2 | 3)
        || machine != if wide { 62 } else { 3 }
        || word(file, 20, 32)? != 1
        || word(file, if wide { 48 } else { 36 }, 32)? != 0
        || word(file, if wide { 52 } else { 40 }, 16)? != if wide { 64 } else { 52 }
    {
        return Err(FormatError::Unsupported);
    }
    let entry = word(file, 24, bits)?;
    let phoff = word(file, if wide { 32 } else { 28 }, bits)?;
    let shoff = word(file, if wide { 40 } else { 32 }, bits)?;
    let phstride = word(file, if wide { 54 } else { 42 }, 16)?;
    let shstride = word(file, if wide { 58 } else { 46 }, 16)?;
    let mut phcount = word(file, if wide { 56 } else { 44 }, 16)?;
    let mut shcount = word(file, if wide { 60 } else { 48 }, 16)?;
    let mut string_section = word(file, if wide { 62 } else { 50 }, 16)?;
    if shoff != 0 {
        let zero = data(file, shoff, if wide { 64 } else { 40 })?;
        if shstride != if wide { 64 } else { 40 } || word(zero, 4, 32)? != 0 {
            return Err(FormatError::InvalidRange);
        }
        if shcount == 0 {
            shcount = word(zero, if wide { 32 } else { 20 }, bits)?;
        }
        if phcount == 65535 {
            phcount = word(zero, if wide { 44 } else { 28 }, 32)?;
        }
        if string_section == 65535 {
            string_section = word(zero, if wide { 40 } else { 24 }, 32)?;
        }
    } else if shcount != 0 || string_section != 0 || phcount == 65535 {
        return Err(FormatError::InvalidRange);
    }
    if phcount == 0 || phstride != if wide { 56 } else { 32 } {
        return Err(FormatError::InvalidRecordSize);
    }
    data(
        file,
        phoff,
        usize::try_from(
            phcount
                .checked_mul(phstride)
                .ok_or(FormatError::InvalidRange)?,
        )
        .map_err(|_| FormatError::InvalidRange)?,
    )?;
    data(
        file,
        shoff,
        usize::try_from(
            shcount
                .checked_mul(shstride)
                .ok_or(FormatError::InvalidRange)?,
        )
        .map_err(|_| FormatError::InvalidRange)?,
    )?;
    let mut segments = Vec::new();
    let mut first = u64::MAX;
    let mut end = 0;
    let mut dynamic_segment = None;
    let mut tls_segment = None;
    let mut interpreter = false;
    for i in 0..phcount {
        let p = data(file, phoff + i * phstride, phstride as usize)?;
        let s = Segment {
            kind: word(p, 0, 32)? as u32,
            flags: word(p, if wide { 4 } else { 24 }, 32)? as u32,
            offset: word(p, if wide { 8 } else { 4 }, bits)?,
            address: word(p, if wide { 16 } else { 8 }, bits)?,
            file_bytes: word(p, if wide { 32 } else { 16 }, bits)?,
            memory_bytes: word(p, if wide { 40 } else { 20 }, bits)?,
            alignment: word(p, if wide { 48 } else { 28 }, bits)?,
        };
        if s.kind != 0 {
            data(
                file,
                s.offset,
                usize::try_from(s.file_bytes).map_err(|_| FormatError::InvalidRange)?,
            )?;
        }
        if matches!(s.kind, 1 | 2 | 7)
            && (s.file_bytes > s.memory_bytes
                || s.address.checked_add(s.memory_bytes).is_none()
                || (!wide && s.address + s.memory_bytes > 1 << 32)
                || (s.alignment > 1
                    && (!s.alignment.is_power_of_two()
                        || s.address % s.alignment != s.offset % s.alignment)))
        {
            return Err(FormatError::InvalidRange);
        }
        if s.kind == 1
            && (s.memory_bytes != 0 || (role == LoadRole::Library && s.address % 4096 != 0))
        {
            if s.address % 4096 != s.offset % 4096 {
                return Err(FormatError::InvalidRange);
            }
            first = first.min(s.address & !4095);
            end = end.max(page_end(s.address + s.memory_bytes)?);
        } else if s.kind == 2 {
            if dynamic_segment.replace(s).is_some() {
                return Err(FormatError::InvalidRange);
            }
        } else if s.kind == 7 {
            if tls_segment.replace(s).is_some() {
                return Err(FormatError::InvalidRange);
            }
        } else if s.kind == 3 {
            if interpreter {
                return Err(FormatError::InvalidValue);
            }
            terminated(data(file, s.offset, s.file_bytes as usize)?, 0)?;
            interpreter = true;
        }
        segments.push(s);
    }
    let length = owned_length(end.checked_sub(first).ok_or(FormatError::InvalidRange)?)?;
    let base = requested.unwrap_or(first);
    let bias = base.checked_sub(first).ok_or(FormatError::InvalidRange)?;
    if !bias.is_multiple_of(4096) || (kind == 2 && bias != 0) {
        return Err(FormatError::InvalidRange);
    }
    if base.checked_add(length as u64).is_none()
        || (!wide && base + length as u64 > 1 << 32)
        || segments.iter().any(|s| {
            matches!(s.kind, 1 | 7) && s.alignment > 1 && !bias.is_multiple_of(s.alignment)
        })
    {
        return Err(FormatError::InvalidRange);
    }
    let mut bytes = vec![0; length].into_boxed_slice();
    let mut permissions = vec![0u8; length / 4096];
    for s in &segments {
        if s.kind != 1
            || (s.memory_bytes == 0 && (role != LoadRole::Library || s.address % 4096 == 0))
        {
            continue;
        }
        let prefix = s.address % 4096;
        let file_start = s
            .offset
            .checked_sub(prefix)
            .ok_or(FormatError::InvalidRange)?;
        let begin = s.address - prefix;
        let data_end = s.address + s.file_bytes;
        let allocated_end = s.address + s.memory_bytes;
        let file_end = page_end(data_end)?;
        let limit = page_end(allocated_end)?;
        if (role == LoadRole::Library && file_end > begin)
            || (role == LoadRole::Program && s.file_bytes != 0)
        {
            let count = (file_end - begin) as usize;
            let destination = (begin - first) as usize;
            bytes[destination..destination + count].fill(0);
            let source_count = count.min(
                file.len()
                    .checked_sub(file_start as usize)
                    .ok_or(FormatError::InvalidRange)?,
            );
            bytes[destination..destination + source_count].copy_from_slice(data(
                file,
                file_start,
                source_count,
            )?);
        }
        if allocated_end > data_end {
            let (zero, zero_end) = if role == LoadRole::Program {
                (
                    if s.file_bytes == 0 {
                        begin
                    } else if s.flags & 2 != 0 {
                        data_end
                    } else {
                        file_end
                    },
                    limit,
                )
            } else {
                (
                    data_end,
                    if allocated_end > file_end {
                        limit
                    } else {
                        allocated_end
                    },
                )
            };
            if zero < zero_end {
                bytes[(zero - first) as usize..(zero_end - first) as usize].fill(0);
            }
        }
        permissions[((begin - first) / 4096) as usize..((limit - first) / 4096) as usize]
            .fill(0x80 | (s.flags as u8 & 7));
    }
    let mut regions = Vec::new();
    let mut at = 0;
    while at < permissions.len() {
        let value = permissions[at];
        let start = at;
        while at < permissions.len() && permissions[at] == value {
            at += 1;
        }
        if value & 0x80 != 0 {
            regions.push(Region {
                offset: start * 4096,
                length: (at - start) * 4096,
                read: value & 4 != 0,
                write: value & 2 != 0,
                execute: value & 1 != 0,
            });
        }
    }
    let range = |address: u64, count: usize| {
        mapped(
            &bytes,
            &regions,
            address
                .checked_sub(first)
                .ok_or(FormatError::InvalidRange)?,
            count,
        )
    };
    let file_view = metadata::File {
        bytes: file,
        segments: &segments,
    };
    if entry != 0 {
        range(entry, 1)?;
        if !regions.iter().any(|r| {
            r.execute
                && entry >= first + r.offset as u64
                && entry < first + r.offset as u64 + r.length as u64
        }) {
            return Err(FormatError::InvalidRange);
        }
    }
    // Validate section extents and names even though execution uses PT_LOAD.
    let mut section_names = None;
    let mut section_name_offsets = Vec::new();
    let mut sections = Vec::new();
    for i in 0..shcount {
        let s = data(file, shoff + i * shstride, shstride as usize)?;
        let kind = word(s, 4, 32)?;
        let offset = word(s, if wide { 24 } else { 16 }, bits)?;
        let size = word(s, if wide { 32 } else { 20 }, bits)?;
        if kind != 0 && kind != 8 {
            data(
                file,
                offset,
                usize::try_from(size).map_err(|_| FormatError::InvalidRange)?,
            )?;
        }
        if string_section != 0 && i == string_section {
            if kind != 3 {
                return Err(FormatError::InvalidValue);
            }
            section_names = Some(data(file, offset, size as usize)?);
        }
        section_name_offsets.push(word(s, 0, 32)?);
        sections.push(metadata::Section {
            kind: kind as u32,
            address: word(s, if wide { 16 } else { 12 }, bits)?,
            offset,
            bytes: size,
            link: word(s, if wide { 40 } else { 24 }, 32)? as u32,
            entry_bytes: word(s, if wide { 56 } else { 36 }, bits)?,
        });
    }
    if string_section != 0 {
        let table = section_names.ok_or(FormatError::InvalidRange)?;
        for offset in section_name_offsets {
            terminated(table, offset)?;
        }
    }
    let mut dynamic = Vec::new();
    if let Some(s) = dynamic_segment {
        let width = (bits / 8) as usize;
        let count = usize::try_from(s.file_bytes).map_err(|_| FormatError::InvalidRange)?;
        if count % (width * 2) != 0 {
            return Err(FormatError::InvalidRecordSize);
        }
        range(
            s.address,
            usize::try_from(s.memory_bytes).map_err(|_| FormatError::InvalidRange)?,
        )?;
        let records = file_view.read(s.address, count)?;
        if records != data(file, s.offset, count)? {
            return Err(FormatError::InvalidValue);
        }
        let mut complete = false;
        for d in records.chunks_exact(width * 2) {
            let tag = word(d, 0, bits)?;
            let value = word(d, width as u64, bits)?;
            if tag == 0 {
                complete = true;
                break;
            }
            if ((2..=30).contains(&tag)
                || (32..=37).contains(&tag)
                || matches!(
                    tag,
                    39 | 0x6ffffef5
                        | 0x6ffffff0
                        | 0x6ffffffc
                        | 0x6ffffffd
                        | 0x6ffffffe
                        | 0x6fffffff
                ))
                && dynamic.iter().any(|&(prior, _)| prior == tag)
            {
                return Err(FormatError::InvalidValue);
            }
            dynamic.push((tag, value));
        }
        if !complete {
            return Err(FormatError::InvalidRange);
        }
    }
    let mut raw_names = Vec::new();
    let mut needed = Vec::new();
    for &(tag, value) in &dynamic {
        if matches!(tag, 1 | 14 | 15 | 29) {
            let label = file_view.text(&dynamic, value)?;
            raw_names.push(label);
            if tag == 1 {
                needed.push(label);
            }
        }
    }
    let raw_symbols = file_view.symbols(bits, &sections, &dynamic)?;
    for symbol in &raw_symbols.rows {
        raw_names.push(symbol.name);
    }
    for version in &raw_symbols.versions {
        raw_names.push(version.name);
        if let Some(library) = version.library {
            raw_names.push(library);
        }
    }
    let names = names(&raw_names)?;
    let needed = needed
        .into_iter()
        .map(|label| name(&names, label))
        .collect::<Result<Box<[_]>, _>>()?;
    let symbols = raw_symbols
        .rows
        .into_iter()
        .map(|s| {
            let address =
                if s.section == 0 || s.section == 0xfff1 || s.section == 0xfff2 || s.kind == 6 {
                    s.value
                } else {
                    bias.checked_add(s.value).ok_or(FormatError::InvalidRange)?
                };
            if bits == 32 && address > u64::from(u32::MAX) {
                return Err(FormatError::InvalidRange);
            }
            Ok(Symbol {
                name: Some(name(&names, s.name)?),
                address,
                bytes: s.bytes,
                ordinal: None,
                forward: None,
                defined: s.section != 0,
                absolute: s.section == 0xfff1,
                weak: s.binding == 2,
                section: s.section,
                binding: s.binding,
                kind: s.kind,
                visibility: s.visibility,
                version: s
                    .version
                    .map(|v| {
                        Ok(Version {
                            name: name(&names, v.name)?,
                            library: v.library.map(|library| name(&names, library)).transpose()?,
                            weak: v.weak,
                        })
                    })
                    .transpose()?,
                hidden_version: s.hidden_version,
            })
        })
        .collect::<Result<Box<[_]>, FormatError>>()?;
    let relocations = file_view.relocations(bits, bias, &dynamic, &symbols)?;
    let tls = tls_segment
        .map(|s| {
            if s.file_bytes != 0 {
                file_view.read(
                    s.address,
                    usize::try_from(s.file_bytes).map_err(|_| FormatError::InvalidRange)?,
                )?;
            }
            Ok(Tls {
                address: bias
                    .checked_add(s.address)
                    .ok_or(FormatError::InvalidRange)?,
                file_bytes: usize::try_from(s.file_bytes).map_err(|_| FormatError::InvalidRange)?,
                zero_bytes: usize::try_from(s.memory_bytes - s.file_bytes)
                    .map_err(|_| FormatError::InvalidRange)?,
                index: None,
                alignment: usize::try_from(s.alignment.max(1))
                    .map_err(|_| FormatError::InvalidRange)?,
            })
        })
        .transpose()?;
    let mut relro = Vec::new();
    for s in &segments {
        if s.kind == 0x6474e552 && s.memory_bytes != 0 {
            let count = usize::try_from(s.memory_bytes).map_err(|_| FormatError::InvalidRange)?;
            range(s.address, count)?;
            let begin = s.address & !4095;
            let end = s
                .address
                .checked_add(s.memory_bytes)
                .ok_or(FormatError::InvalidRange)?
                & !4095;
            if end > begin {
                range(
                    begin,
                    usize::try_from(end - begin).map_err(|_| FormatError::InvalidRange)?,
                )?;
            }
            relro.push((bias + s.address, count));
        }
    }
    Ok(Image {
        target: Target {
            bits,
            encoding: Encoding::Elf,
        },
        preferred_base: first,
        base,
        entry: if entry == 0 { 0 } else { bias + entry },
        bytes,
        regions: regions.into_boxed_slice(),
        names,
        symbols,
        imports: Box::new([]),
        relocations: relocations.into_boxed_slice(),
        needed,
        tls,
        initializers: Box::new([]),
        dynamic: dynamic.into_boxed_slice(),
        relro: relro.into_boxed_slice(),
    })
}
