//! PACK/ZIP offsets mined from muse-final:crates/content/src/archive.rs.
//! Native contracts: Q1/Q2 pack headers and Q3 qcommon/unzip.c.
use crate::FormatError;
use qa_platform::files::ReadFile;
use std::{io, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveKind {
    Pak,
    Zip,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    Stored,
    Deflate,
}

pub struct ArchiveEntry {
    pub name: Box<[u8]>,
    pub directory: bool,
    pub length: u64,
    pub compressed_length: u64,
    pub crc32: Option<u32>,
    pub compression: Compression,
    offset: u64,
}

pub struct Archive {
    pub kind: ArchiveKind,
    pub entries: Box<[ArchiveEntry]>,
    file: Arc<ReadFile>,
}

/// Caller-owned inflate state, allocated once at load and reset between files.
pub struct ArchiveReader {
    cursor: ArchiveCursor,
    output: [u8; 8192],
}
struct ArchiveCursor {
    member: Option<(Arc<ReadFile>, usize)>,
    decoder: flate2::Decompress,
    input: [u8; 8192],
    first: usize,
    last: usize,
    fetched: u64,
    position: u64,
    crc: crc32fast::Hasher,
    done: bool,
}
impl Default for ArchiveReader {
    fn default() -> Self {
        Self {
            cursor: ArchiveCursor {
                member: None,
                decoder: flate2::Decompress::new(false),
                input: [0; 8192],
                first: 0,
                last: 0,
                fetched: 0,
                position: 0,
                crc: crc32fast::Hasher::new(),
                done: false,
            },
            output: [0; 8192],
        }
    }
}
impl ArchiveReader {
    fn start(&mut self, archive: &Archive, entry: usize) {
        self.cursor.member = Some((Arc::clone(&archive.file), entry));
        self.cursor.decoder.reset(false);
        self.cursor.first = 0;
        self.cursor.last = 0;
        self.cursor.fetched = 0;
        self.cursor.position = 0;
        self.cursor.crc = crc32fast::Hasher::new();
        self.cursor.done = false;
    }
    /// Progress of this reader's current member, useful for source diagnostics.
    pub fn stream_position(&self) -> u64 {
        self.cursor.position
    }
    pub fn close_stream(&mut self) {
        self.cursor.member = None;
    }
}

fn range(size: u64, start: u64, length: u64) -> Result<(), FormatError> {
    if start.checked_add(length).is_none_or(|end| end > size) {
        Err(FormatError::InvalidRange)
    } else {
        Ok(())
    }
}
fn short(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}
fn word(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}
impl FormatError {
    pub fn from_io(error: io::Error) -> Self {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            Self::Truncated
        } else {
            Self::Io(error.kind())
        }
    }
}
fn bytes(file: &ReadFile, size: u64, at: u64, length: u64) -> Result<Vec<u8>, FormatError> {
    range(size, at, length)?;
    if length > 64 * 1024 * 1024 {
        return Err(FormatError::InvalidRange);
    }
    let mut out = vec![0; length as usize];
    file.read_exact_at(at, &mut out)
        .map_err(FormatError::from_io)?;
    Ok(out)
}

impl Archive {
    pub fn find(&self, name: &[u8]) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| entry.name.as_ref() == name)
    }
    pub fn parse(file: Arc<ReadFile>) -> Result<Self, FormatError> {
        let size = file.length().map_err(FormatError::from_io)?;
        let header = bytes(&file, size, 0, 12)?;
        if &header[..4] == b"PACK" {
            Self::pak(file, size, &header)
        } else {
            Self::zip(file, size)
        }
    }

    fn pak(file: Arc<ReadFile>, size: u64, header: &[u8]) -> Result<Self, FormatError> {
        let offset = u64::from(word(header, 4));
        let length = u64::from(word(header, 8));
        if offset < 12
            || offset > i32::MAX as u64
            || length > i32::MAX as u64
            || !length.is_multiple_of(64)
        {
            return Err(FormatError::InvalidRecordSize);
        }
        let directory = bytes(&file, size, offset, length)?;
        let mut entries = Vec::with_capacity(directory.len() / 64);
        for record in directory.as_chunks::<64>().0 {
            let name = qa_core::text::until_nul(&record[..56]);
            let offset = u64::from(word(record, 56));
            let length = u64::from(word(record, 60));
            if offset > i32::MAX as u64 || length > i32::MAX as u64 {
                return Err(FormatError::InvalidRange);
            }
            range(size, offset, length)?;
            entries.push(ArchiveEntry {
                name: name.into(),
                directory: name.ends_with(b"/"),
                offset,
                length,
                compressed_length: length,
                compression: Compression::Stored,
                crc32: None,
            });
        }
        Ok(Self {
            kind: ArchiveKind::Pak,
            entries: entries.into_boxed_slice(),
            file,
        })
    }

    fn zip(file: Arc<ReadFile>, size: u64) -> Result<Self, FormatError> {
        let tail = bytes(&file, size, size.saturating_sub(65557), size.min(65557))?;
        let end = (0..=tail.len().checked_sub(22).ok_or(FormatError::Truncated)?)
            .rev()
            .find(|&at| {
                word(&tail, at) == 0x06054b50
                    && at + 22 + usize::from(short(&tail, at + 20)) == tail.len()
            })
            .ok_or(FormatError::Unsupported)?;
        let record = &tail[end..];
        let count = short(record, 10);
        let length = u64::from(word(record, 12));
        let relative = u64::from(word(record, 16));
        if short(record, 4) != 0
            || short(record, 6) != 0
            || short(record, 8) != count
            || count == u16::MAX
            || length == u32::MAX as u64
            || relative == u32::MAX as u64
        {
            return Err(FormatError::Unsupported);
        }
        let end_offset = size - tail.len() as u64 + end as u64;
        let prefix = end_offset
            .checked_sub(relative)
            .and_then(|v| v.checked_sub(length))
            .ok_or(FormatError::InvalidRange)?;
        let central = relative + prefix;
        let directory = bytes(&file, end_offset, central, length)?;
        let mut entries = Vec::with_capacity(usize::from(count));
        let mut at = 0usize;
        for _ in 0..count {
            let header = directory
                .get(at..at.checked_add(46).ok_or(FormatError::InvalidRange)?)
                .ok_or(FormatError::Truncated)?;
            if word(header, 0) != 0x02014b50 {
                return Err(FormatError::InvalidRecordSize);
            }
            let flags = short(header, 8);
            let compression = match short(header, 10) {
                0 => Compression::Stored,
                8 => Compression::Deflate,
                _ => return Err(FormatError::Unsupported),
            };
            let crc = word(header, 16);
            let compressed = u64::from(word(header, 20));
            let length = u64::from(word(header, 24));
            let names = usize::from(short(header, 28));
            let extra = usize::from(short(header, 30));
            let comment = usize::from(short(header, 32));
            let local_offset = u64::from(word(header, 42));
            if flags & 1 != 0
                || short(header, 34) != 0
                || [compressed, length, local_offset].contains(&(u32::MAX as u64))
            {
                return Err(FormatError::Unsupported);
            }
            if compression == Compression::Stored && compressed != length {
                return Err(FormatError::InvalidRecordSize);
            }
            let name = directory
                .get(at + 46..at + 46 + names)
                .ok_or(FormatError::Truncated)?;
            at = at
                .checked_add(46 + names + extra + comment)
                .filter(|&end| end <= directory.len())
                .ok_or(FormatError::Truncated)?;
            let local_at = local_offset + prefix;
            let local = bytes(&file, central, local_at, 30)?;
            let local_flags = short(&local, 6);
            let descriptor = local_flags & 8 != 0;
            if word(&local, 0) != 0x04034b50
                || (local_flags ^ flags) & 0x7fff != 0
                || short(&local, 8) != short(header, 10)
                || ((!descriptor || word(&local, 14) != 0) && word(&local, 14) != crc)
                || ((!descriptor || word(&local, 18) != 0)
                    && u64::from(word(&local, 18)) != compressed)
                || ((!descriptor || word(&local, 22) != 0) && u64::from(word(&local, 22)) != length)
            {
                return Err(FormatError::InvalidRecordSize);
            }
            let local_names = u64::from(short(&local, 26));
            let local_extra = u64::from(short(&local, 28));
            let variable = bytes(&file, central, local_at + 30, local_names + local_extra)?;
            if variable.get(..local_names as usize) != Some(name) {
                return Err(FormatError::InvalidRecordSize);
            }
            let offset = local_at + 30 + local_names + local_extra;
            range(central, offset, compressed)?;
            entries.push(ArchiveEntry {
                name: name.into(),
                directory: name.ends_with(b"/"),
                offset,
                length,
                compressed_length: compressed,
                compression,
                crc32: Some(crc),
            });
        }
        if at != directory.len() {
            return Err(FormatError::InvalidRecordSize);
        }
        Ok(Self {
            kind: ArchiveKind::Zip,
            entries: entries.into_boxed_slice(),
            file,
        })
    }

    pub fn read_into_reusing(
        &self,
        index: usize,
        destination: &mut [u8],
        reader: &mut ArchiveReader,
    ) -> Result<usize, FormatError> {
        let entry = self.entries.get(index).ok_or(FormatError::InvalidRange)?;
        let length = usize::try_from(entry.length).map_err(|_| FormatError::InvalidRange)?;
        let out = destination
            .get_mut(..length)
            .ok_or(FormatError::InvalidRange)?;
        self.read_range_reusing(index, 0, out, reader)
    }

    /// Module range reads retain their full unread-tail CRC verification.
    pub fn read_range_reusing(
        &self,
        index: usize,
        offset: u64,
        destination: &mut [u8],
        reader: &mut ArchiveReader,
    ) -> Result<usize, FormatError> {
        let entry = self.entries.get(index).ok_or(FormatError::InvalidRange)?;
        let remaining = entry
            .length
            .checked_sub(offset)
            .ok_or(FormatError::InvalidRange)?;
        let count = destination
            .len()
            .min(remaining.try_into().unwrap_or(usize::MAX));
        if entry.compression == Compression::Stored {
            self.read_stored(entry, offset, &mut destination[..count])?;
            if offset == 0
                && count as u64 == entry.length
                && entry
                    .crc32
                    .is_some_and(|crc| crc32fast::hash(&destination[..count]) != crc)
            {
                return Err(FormatError::Checksum);
            }
            return Ok(count);
        }
        reader.start(self, index);
        let count = self.read_stream_reusing(index, offset, destination, reader)?;
        while !reader.cursor.done {
            self.pump(entry, &mut reader.cursor, &mut reader.output)?;
        }
        Ok(count)
    }

    /// Consecutive forward requests advance the existing decoder and CRC once.
    /// A different member or backward seek resets it. CRC is checked at EOF;
    /// callers requiring unread-tail validation use read_range_reusing instead.
    pub fn read_stream_reusing(
        &self,
        index: usize,
        offset: u64,
        destination: &mut [u8],
        reader: &mut ArchiveReader,
    ) -> Result<usize, FormatError> {
        let entry = self.entries.get(index).ok_or(FormatError::InvalidRange)?;
        let remaining = entry
            .length
            .checked_sub(offset)
            .ok_or(FormatError::InvalidRange)?;
        if reader
            .cursor
            .member
            .as_ref()
            .is_none_or(|(file, member)| !Arc::ptr_eq(file, &self.file) || *member != index)
            || offset < reader.cursor.position
        {
            reader.start(self, index);
        }
        while reader.cursor.position < offset {
            let skip = (offset - reader.cursor.position).min(reader.output.len() as u64) as usize;
            self.pump(entry, &mut reader.cursor, &mut reader.output[..skip])?;
        }
        let count = destination
            .len()
            .min(remaining.try_into().unwrap_or(usize::MAX));
        let mut filled = 0;
        while filled < count {
            filled += self.pump(entry, &mut reader.cursor, &mut destination[filled..count])?;
        }
        if reader.cursor.position == entry.length && !reader.cursor.done {
            // An exact-size output may leave the terminal deflate marker unread.
            self.pump(entry, &mut reader.cursor, &mut reader.output[..1])?;
        }
        Ok(count)
    }

    fn read_stored(
        &self,
        entry: &ArchiveEntry,
        offset: u64,
        out: &mut [u8],
    ) -> Result<(), FormatError> {
        self.file
            .read_exact_at(entry.offset + offset, out)
            .map_err(FormatError::from_io)
    }

    fn pump(
        &self,
        entry: &ArchiveEntry,
        cursor: &mut ArchiveCursor,
        out: &mut [u8],
    ) -> Result<usize, FormatError> {
        if cursor.done || out.is_empty() {
            return Ok(0);
        }
        let produced = if entry.compression == Compression::Stored {
            let count = out.len().min(
                (entry.length - cursor.position)
                    .try_into()
                    .unwrap_or(usize::MAX),
            );
            self.read_stored(entry, cursor.position, &mut out[..count])?;
            cursor.position += count as u64;
            cursor.done = cursor.position == entry.length;
            count
        } else {
            loop {
                if cursor.first == cursor.last && cursor.fetched < entry.compressed_length {
                    let count = cursor
                        .input
                        .len()
                        .min((entry.compressed_length - cursor.fetched) as usize);
                    self.file
                        .read_exact_at(entry.offset + cursor.fetched, &mut cursor.input[..count])
                        .map_err(FormatError::from_io)?;
                    cursor.first = 0;
                    cursor.last = count;
                    cursor.fetched += count as u64;
                }
                let before_in = cursor.decoder.total_in();
                let before_out = cursor.decoder.total_out();
                let status = cursor
                    .decoder
                    .decompress(
                        &cursor.input[cursor.first..cursor.last],
                        out,
                        flate2::FlushDecompress::None,
                    )
                    .map_err(|_| FormatError::Compression)?;
                let consumed = cursor.decoder.total_in() - before_in;
                let produced = (cursor.decoder.total_out() - before_out) as usize;
                cursor.first += consumed as usize;
                cursor.position = cursor.decoder.total_out();
                if cursor.position > entry.length {
                    return Err(FormatError::Compression);
                }
                if status == flate2::Status::StreamEnd {
                    if cursor.decoder.total_in() != entry.compressed_length
                        || cursor.position != entry.length
                    {
                        return Err(FormatError::Compression);
                    }
                    cursor.done = true;
                }
                if produced != 0 || cursor.done {
                    break produced;
                }
                if consumed == 0 {
                    return Err(FormatError::Compression);
                }
            }
        };
        cursor.crc.update(&out[..produced]);
        if cursor.done
            && entry
                .crc32
                .is_some_and(|expected| cursor.crc.clone().finalize() != expected)
        {
            // A failed stream cannot subsequently turn into an accepted EOF.
            cursor.member = None;
            return Err(FormatError::Checksum);
        }
        Ok(produced)
    }
}
