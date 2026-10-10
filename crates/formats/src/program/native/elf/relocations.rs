use super::{metadata::File, metadata::value, *};

impl File<'_> {
    fn relocation(
        &self,
        bits: u32,
        bias: u64,
        address: u64,
        kind: u32,
        index: usize,
        addend: Option<i64>,
        symbols: &[Symbol],
    ) -> Result<Relocation, FormatError> {
        let symbol = if index == 0 {
            None
        } else {
            Some(symbols.get(index).ok_or(FormatError::InvalidReference(
                "ELF relocation symbol",
                index,
            ))?)
        };
        let bytes = match (bits, kind) {
            (_, 0) => 0,
            (_, 5) => usize::try_from(
                symbol
                    .ok_or(FormatError::InvalidReference("ELF COPY symbol", index))?
                    .bytes,
            )
            .map_err(|_| FormatError::InvalidRange)?,
            (32, 1 | 2 | 6 | 7 | 8 | 9 | 10 | 14 | 17 | 34 | 35 | 36 | 37 | 38 | 42) => 4,
            (64, 1 | 6 | 7 | 8 | 16 | 17 | 18 | 24 | 33 | 37 | 38) => 8,
            (64, 2 | 10 | 11 | 23 | 32) => 4,
            (64, 12 | 13) => 2,
            (64, 14 | 15) => 1,
            _ => {
                return Err(FormatError::InvalidReference(
                    "ELF relocation type",
                    kind as usize,
                ));
            }
        };
        if index != 0
            && (kind == 8 || (bits == 64 && matches!(kind, 37 | 38)) || (bits == 32 && kind == 42))
        {
            return Err(FormatError::InvalidReference("ELF relative symbol", index));
        }
        if kind != 0
            && !self.segments.iter().any(|s| {
                s.kind == 1
                    && address >= s.address
                    && address - s.address <= s.memory_bytes
                    && bytes as u64 <= s.memory_bytes - (address - s.address)
            })
        {
            return Err(FormatError::InvalidRange);
        }
        // R_NONE has no address operation; its otherwise unused offset is inert.
        let address = if kind == 0 {
            address
        } else {
            address.checked_add(bias).ok_or(FormatError::InvalidRange)?
        };
        if kind != 0
            && bits == 32
            && address
                .checked_add(bytes as u64)
                .is_none_or(|end| end > 1 << 32)
        {
            return Err(FormatError::InvalidRange);
        }
        Ok(Relocation {
            address,
            bytes,
            kind,
            symbol: (index != 0).then_some(index),
            addend,
        })
    }
    pub fn relocations(
        &self,
        bits: u32,
        bias: u64,
        dynamic: &[(u64, u64)],
        symbols: &[Symbol],
    ) -> Result<Vec<Relocation>, FormatError> {
        let width = u64::from(bits / 8);
        let mut tables = Vec::new();
        let mut relocations = Vec::new();
        for pass in 0..3 {
            let address = value(dynamic, [17, 7, 23][pass]);
            let length = value(dynamic, [18, 8, 2][pass]);
            if address.is_none() && length.is_none() {
                continue;
            }
            let rela = if pass == 2 {
                match value(dynamic, 20) {
                    Some(7) => true,
                    Some(17) => false,
                    _ => return Err(FormatError::InvalidValue),
                }
            } else {
                pass == 1
            };
            let stride = width * if rela { 3 } else { 2 };
            if pass != 2 && value(dynamic, if rela { 9 } else { 19 }) != Some(stride) {
                return Err(FormatError::InvalidRecordSize);
            }
            let address = address.ok_or(FormatError::InvalidRange)?;
            let length = length.ok_or(FormatError::InvalidRange)?;
            if length % stride != 0 {
                return Err(FormatError::InvalidRecordSize);
            }
            if tables.contains(&(address, length, rela)) {
                continue;
            }
            tables.push((address, length, rela));
            if length == 0 {
                continue;
            }
            let table = self.read(
                address,
                usize::try_from(length).map_err(|_| FormatError::InvalidRange)?,
            )?;
            for p in table.chunks_exact(stride as usize) {
                let info = word(p, width, bits)?;
                let kind = if bits == 64 {
                    info as u32
                } else {
                    (info & 255) as u32
                };
                let index = usize::try_from(info >> if bits == 64 { 32 } else { 8 })
                    .map_err(|_| FormatError::InvalidRange)?;
                let addend = if rela {
                    let word = word(p, width * 2, bits)?;
                    Some(if bits == 64 {
                        word as i64
                    } else {
                        i64::from(word as u32 as i32)
                    })
                } else {
                    None
                };
                relocations.push(self.relocation(
                    bits,
                    bias,
                    word(p, 0, bits)?,
                    kind,
                    index,
                    addend,
                    symbols,
                )?);
            }
        }
        let address = value(dynamic, 36);
        let length = value(dynamic, 35);
        let stride = value(dynamic, 37);
        if address.is_some() || length.is_some() || stride.is_some() {
            let address = address.ok_or(FormatError::InvalidRange)?;
            let length = length.ok_or(FormatError::InvalidRange)?;
            if stride != Some(width) || length % width != 0 {
                return Err(FormatError::InvalidRecordSize);
            }
            if length != 0 {
                let table = self.read(
                    address,
                    usize::try_from(length).map_err(|_| FormatError::InvalidRange)?,
                )?;
                let mut cursor = None;
                for p in table.chunks_exact(width as usize) {
                    let record = word(p, 0, bits)?;
                    if record & 1 == 0 {
                        relocations.push(self.relocation(bits, bias, record, 8, 0, None, symbols)?);
                        cursor = Some(record.checked_add(width).ok_or(FormatError::InvalidRange)?);
                    } else {
                        let base = cursor.ok_or(FormatError::InvalidRange)?;
                        let end = base
                            .checked_add((u64::from(bits) - 1) * width)
                            .ok_or(FormatError::InvalidRange)?;
                        if bits == 32 && end > 1 << 32 {
                            return Err(FormatError::InvalidRange);
                        }
                        for bit in 1..bits {
                            if record & (1u64 << bit) != 0 {
                                relocations.push(self.relocation(
                                    bits,
                                    bias,
                                    base + u64::from(bit - 1) * width,
                                    8,
                                    0,
                                    None,
                                    symbols,
                                )?);
                            }
                        }
                        cursor = Some(end);
                    }
                }
            }
        }
        Ok(relocations)
    }
}
