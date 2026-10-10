use super::*;

pub(super) struct File<'a> {
    pub bytes: &'a [u8],
    pub segments: &'a [Segment],
}
#[derive(Clone, Copy)]
pub(super) struct Section {
    pub kind: u32,
    pub address: u64,
    pub offset: u64,
    pub bytes: u64,
    pub link: u32,
    pub entry_bytes: u64,
}
#[derive(Clone, Copy)]
pub(super) struct RawVersion<'a> {
    pub index: u16,
    pub name: &'a [u8],
    pub library: Option<&'a [u8]>,
    pub weak: bool,
}
pub(super) struct RawSymbol<'a> {
    pub name: &'a [u8],
    pub value: u64,
    pub bytes: u64,
    pub section: u32,
    pub binding: u8,
    pub kind: u8,
    pub visibility: u8,
    pub version: Option<RawVersion<'a>>,
    pub hidden_version: bool,
}
pub(super) struct Symbols<'a> {
    pub rows: Vec<RawSymbol<'a>>,
    pub versions: Vec<RawVersion<'a>>,
}
pub(super) fn value(dynamic: &[(u64, u64)], tag: u64) -> Option<u64> {
    dynamic.iter().find(|&&(t, _)| t == tag).map(|&(_, v)| v)
}
fn count(value: u64) -> Result<usize, FormatError> {
    usize::try_from(value).map_err(|_| FormatError::InvalidRange)
}
fn advance(address: u64, next: u64, required: bool) -> Result<u64, FormatError> {
    if required && next == 0 {
        return Err(FormatError::InvalidRange);
    }
    address.checked_add(next).ok_or(FormatError::InvalidRange)
}
impl<'a> File<'a> {
    pub fn read(&self, address: u64, length: usize) -> Result<&'a [u8], FormatError> {
        let s = self
            .segments
            .iter()
            .find(|s| {
                s.kind == 1
                    && address >= s.address
                    && address - s.address <= s.file_bytes
                    && length as u64 <= s.file_bytes - (address - s.address)
            })
            .ok_or(FormatError::InvalidRange)?;
        data(self.bytes, s.offset + (address - s.address), length)
    }
    pub fn text(&self, dynamic: &[(u64, u64)], index: u64) -> Result<&'a [u8], FormatError> {
        let strings = value(dynamic, 5).ok_or(FormatError::InvalidRange)?;
        let length = value(dynamic, 10).ok_or(FormatError::InvalidRange)?;
        if index >= length {
            return Err(FormatError::InvalidRange);
        }
        terminated(
            self.read(advance(strings, index, false)?, count(length - index)?)?,
            0,
        )
    }
    fn versions(&self, dynamic: &[(u64, u64)]) -> Result<Vec<RawVersion<'a>>, FormatError> {
        let mut versions = Vec::<RawVersion<'a>>::new();
        for needed in [false, true] {
            let address = value(dynamic, if needed { 0x6ffffffe } else { 0x6ffffffc });
            let rows = value(dynamic, if needed { 0x6fffffff } else { 0x6ffffffd });
            let (mut address, rows) = match (address, rows) {
                (None, None | Some(0)) => continue,
                (Some(at), Some(n)) if n != 0 => (at, n),
                _ => return Err(FormatError::InvalidRange),
            };
            if rows > (self.bytes.len() / if needed { 16 } else { 20 }) as u64 {
                return Err(FormatError::InvalidRange);
            }
            for row in 0..rows {
                let p = self.read(address, if needed { 16 } else { 20 })?;
                if word(p, 0, 16)? != 1 {
                    return Err(FormatError::InvalidValue);
                }
                let n = word(p, if needed { 2 } else { 6 }, 16)?;
                if n == 0 {
                    return Err(FormatError::InvalidRange);
                }
                let library = if needed {
                    Some(self.text(dynamic, word(p, 4, 32)?)?)
                } else {
                    None
                };
                let mut auxiliary =
                    advance(address, word(p, if needed { 8 } else { 12 }, 32)?, true)?;
                for i in 0..n {
                    let a = self.read(auxiliary, if needed { 16 } else { 8 })?;
                    if needed || i == 0 {
                        let index =
                            word(if needed { a } else { p }, if needed { 6 } else { 4 }, 16)?
                                as u16
                                & 0x7fff;
                        if versions.iter().any(|v| v.index == index) {
                            return Err(FormatError::InvalidValue);
                        }
                        versions.push(RawVersion {
                            index,
                            name: self.text(dynamic, word(a, if needed { 8 } else { 0 }, 32)?)?,
                            library,
                            weak: word(if needed { a } else { p }, if needed { 4 } else { 2 }, 16)?
                                & 2
                                != 0,
                        });
                    }
                    auxiliary = advance(
                        auxiliary,
                        word(a, if needed { 12 } else { 4 }, 32)?,
                        i + 1 < n,
                    )?;
                }
                address = advance(
                    address,
                    word(p, if needed { 12 } else { 16 }, 32)?,
                    row + 1 < rows,
                )?;
            }
        }
        Ok(versions)
    }
    fn sysv_count(&self, address: u64) -> Result<usize, FormatError> {
        let p = self.read(address, 8)?;
        let buckets = word(p, 0, 32)?;
        let chains = word(p, 4, 32)?;
        if buckets == 0 || chains == 0 {
            return Err(FormatError::InvalidRange);
        }
        let bytes = count(
            (buckets + chains)
                .checked_mul(4)
                .ok_or(FormatError::InvalidRange)?,
        )?;
        let table = self.read(advance(address, 8, false)?, bytes)?;
        for offset in (0..table.len()).step_by(4) {
            if word(table, offset as u64, 32)? >= chains {
                return Err(FormatError::InvalidReference(
                    "ELF symbol bucket",
                    offset / 4,
                ));
            }
        }
        count(chains)
    }
    fn gnu_count(&self, address: u64, bits: u32) -> Result<usize, FormatError> {
        let p = self.read(address, 16)?;
        let buckets = word(p, 0, 32)?;
        let first = word(p, 4, 32)?;
        let bloom = word(p, 8, 32)?;
        if buckets == 0 || bloom == 0 {
            return Err(FormatError::InvalidRange);
        }
        let skip = 16 + bloom * u64::from(bits / 8);
        self.read(address, count(skip)?)?;
        let bucket_address = advance(address, skip, false)?;
        let table = self.read(bucket_address, count(buckets * 4)?)?;
        let chains = advance(bucket_address, buckets * 4, false)?;
        let mut symbols = first;
        for offset in (0..table.len()).step_by(4) {
            let mut symbol = word(table, offset as u64, 32)?;
            if symbol == 0 {
                continue;
            }
            if symbol < first {
                return Err(FormatError::InvalidRange);
            }
            loop {
                if symbol > u64::from(u32::MAX) {
                    return Err(FormatError::InvalidRange);
                }
                let entry = self.read(advance(chains, (symbol - first) * 4, false)?, 4)?;
                symbols = symbols.max(symbol + 1);
                symbol += 1;
                if word(entry, 0, 32)? & 1 != 0 {
                    break;
                }
            }
        }
        count(symbols)
    }
    pub fn symbols(
        &self,
        bits: u32,
        sections: &[Section],
        dynamic: &[(u64, u64)],
    ) -> Result<Symbols<'a>, FormatError> {
        let stride = if bits == 64 { 24 } else { 16 };
        let address = value(dynamic, 6);
        let candidates = sections.iter().enumerate().filter(|(_, s)| match address {
            Some(a) => s.kind == 11 && s.address == a,
            None => s.kind == 2,
        });
        let mut section = None;
        for pair in candidates {
            if section.replace(pair).is_some() {
                return Err(FormatError::InvalidValue);
            }
        }
        let versions = self.versions(dynamic)?;
        let (table, n, strings) = if let Some(address) = address {
            if value(dynamic, 11) != Some(stride) {
                return Err(FormatError::InvalidRecordSize);
            }
            let sysv = value(dynamic, 4).map(|a| self.sysv_count(a)).transpose()?;
            let gnu = value(dynamic, 0x6ffffef5)
                .map(|a| self.gnu_count(a, bits))
                .transpose()?;
            let n = if let Some(bytes) = value(dynamic, 39) {
                if bytes % stride != 0 {
                    return Err(FormatError::InvalidRecordSize);
                }
                count(bytes / stride)?
            } else if let Some(n) = sysv.or(gnu) {
                n
            } else if let Some((_, s)) = section {
                if s.bytes % stride != 0 {
                    return Err(FormatError::InvalidRecordSize);
                }
                count(s.bytes / stride)?
            } else {
                return Err(FormatError::InvalidRange);
            };
            if sysv.is_some_and(|actual| actual != n) || gnu.is_some_and(|actual| actual != n) {
                return Err(FormatError::InvalidValue);
            }
            (
                self.read(
                    address,
                    n.checked_mul(stride as usize)
                        .ok_or(FormatError::InvalidRange)?,
                )?,
                n,
                None,
            )
        } else if let Some((_, s)) = section {
            if s.bytes % stride != 0 {
                return Err(FormatError::InvalidRecordSize);
            }
            let strings = sections
                .get(s.link as usize)
                .filter(|s| s.kind == 3)
                .ok_or(FormatError::InvalidRange)?;
            (
                data(self.bytes, s.offset, count(s.bytes)?)?,
                count(s.bytes / stride)?,
                Some(data(self.bytes, strings.offset, count(strings.bytes)?)?),
            )
        } else {
            return Ok(Symbols {
                rows: Vec::new(),
                versions,
            });
        };
        if section.is_some_and(|(_, s)| s.entry_bytes != stride || s.bytes != n as u64 * stride) {
            return Err(FormatError::InvalidRecordSize);
        }
        let version_table = value(dynamic, 0x6ffffff0)
            .map(|a| self.read(a, n.checked_mul(2).ok_or(FormatError::InvalidRange)?))
            .transpose()?;
        let extended = if let Some(a) = value(dynamic, 34).filter(|_| address.is_some()) {
            Some(self.read(a, n.checked_mul(4).ok_or(FormatError::InvalidRange)?)?)
        } else if let Some((index, _)) = section {
            let mut extended = None;
            for s in sections
                .iter()
                .filter(|s| s.kind == 18 && s.link as usize == index)
            {
                if extended.is_some() || s.entry_bytes != 4 || s.bytes != n as u64 * 4 {
                    return Err(FormatError::InvalidRange);
                }
                extended = Some(data(self.bytes, s.offset, count(s.bytes)?)?);
            }
            extended
        } else {
            None
        };
        let mut rows = Vec::with_capacity(n);
        for (i, p) in table.chunks_exact(stride as usize).enumerate() {
            let name_index = word(p, 0, 32)?;
            let name = match strings {
                Some(strings) => terminated(strings, name_index)?,
                None => self.text(dynamic, name_index)?,
            };
            let info = p[if bits == 64 { 4 } else { 12 }];
            let version_word = version_table
                .map(|v| word(v, i as u64 * 2, 16))
                .transpose()?
                .unwrap_or(1) as u16;
            let version_index = version_word & 0x7fff;
            let version = if version_index > 1 {
                versions.iter().find(|v| v.index == version_index).copied()
            } else {
                None
            };
            if version_index > 1 && version.is_none() {
                return Err(FormatError::InvalidReference("ELF symbol version", i));
            }
            let mut section = word(p, if bits == 64 { 6 } else { 14 }, 16)? as u32;
            if section == 0xffff {
                section =
                    word(extended.ok_or(FormatError::InvalidRange)?, i as u64 * 4, 32)? as u32;
            }
            rows.push(RawSymbol {
                name,
                value: word(p, if bits == 64 { 8 } else { 4 }, bits)?,
                bytes: word(p, if bits == 64 { 16 } else { 8 }, bits)?,
                section,
                binding: if version_index == 0 && section != 0 {
                    0
                } else {
                    info >> 4
                },
                kind: info & 15,
                visibility: p[if bits == 64 { 5 } else { 13 }] & 3,
                version,
                hidden_version: version_word & 0x8000 != 0,
            });
        }
        Ok(Symbols { rows, versions })
    }
}
