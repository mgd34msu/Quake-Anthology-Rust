//! PACK/ZIP offsets mined from muse-final:crates/content/src/archive.rs.
//! Native contracts: Q1/Q2 pack headers and Q3 qcommon/unzip.c.
use crate::FormatError;
use std::{
    fs::File,
    io::{self, Read},
    sync::Arc,
};

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
    file: Arc<File>,
}

/// Caller-owned inflate state, allocated once at load and reset between files.
pub struct ArchiveReader {
    decoder: flate2::Decompress,
    input: [u8; 8192],
    output: [u8; 8192],
}
impl Default for ArchiveReader {
    fn default() -> Self {
        Self {
            decoder: flate2::Decompress::new(false),
            input: [0; 8192],
            output: [0; 8192],
        }
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
fn io_error(error: io::Error) -> FormatError {
    FormatError::Io(error.kind())
}

#[cfg(unix)]
fn read_at(file: &File, out: &mut [u8], at: u64) -> io::Result<usize> {
    use std::os::unix::fs::FileExt;
    file.read_at(out, at)
}
#[cfg(windows)]
fn read_at(file: &File, out: &mut [u8], at: u64) -> io::Result<usize> {
    use std::os::windows::fs::FileExt;
    file.seek_read(out, at)
}
fn exact(file: &File, mut at: u64, mut out: &mut [u8]) -> Result<(), FormatError> {
    while !out.is_empty() {
        let count = match read_at(file, out, at) {
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(io_error(error)),
        };
        if count == 0 {
            return Err(FormatError::Truncated);
        }
        at += count as u64;
        out = &mut out[count..];
    }
    Ok(())
}
fn bytes(file: &File, size: u64, at: u64, length: u64) -> Result<Vec<u8>, FormatError> {
    range(size, at, length)?;
    if length > 64 * 1024 * 1024 {
        return Err(FormatError::InvalidRange);
    }
    let mut out = vec![0; length as usize];
    exact(file, at, &mut out)?;
    Ok(out)
}

impl Archive {
    pub fn find(&self, name: &[u8]) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| entry.name.as_ref() == name)
    }
    pub fn parse(file: Arc<File>) -> Result<Self, FormatError> {
        let size = file.metadata().map_err(io_error)?.len();
        let header = bytes(&file, size, 0, 12)?;
        if &header[..4] == b"PACK" {
            Self::pak(file, size, &header)
        } else {
            Self::zip(file, size)
        }
    }

    fn pak(file: Arc<File>, size: u64, header: &[u8]) -> Result<Self, FormatError> {
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
            let end = record[..56].iter().position(|&b| b == 0).unwrap_or(56);
            let name = &record[..end];
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

    fn zip(file: Arc<File>, size: u64) -> Result<Self, FormatError> {
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

    /// Whole-member read into an admitted caller buffer. No snapshot/digest cache.
    pub fn read_into(&self, index: usize, destination: &mut [u8]) -> Result<usize, FormatError> {
        let entry = self.entries.get(index).ok_or(FormatError::InvalidRange)?;
        if entry.compression == Compression::Stored {
            let length = usize::try_from(entry.length).map_err(|_| FormatError::InvalidRange)?;
            let out = destination
                .get_mut(..length)
                .ok_or(FormatError::InvalidRange)?;
            exact(&self.file, entry.offset, out)?;
            if entry.crc32.is_some_and(|crc| crc32fast::hash(out) != crc) {
                return Err(FormatError::Checksum);
            }
            return Ok(length);
        }
        self.read_into_reusing(index, destination, &mut ArchiveReader::default())
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

    /// Module file handles can read part of a deflated member. The same decoder
    /// traverses it into fixed discard storage and the requested destination;
    /// no second reader, full-member allocation or unchecked ZIP CRC is needed.
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
            .min(usize::try_from(remaining).unwrap_or(usize::MAX));
        let out = &mut destination[..count];
        match entry.compression {
            Compression::Stored => {
                exact(&self.file, entry.offset + offset, out)?;
                if offset == 0
                    && count as u64 == entry.length
                    && entry.crc32.is_some_and(|crc| crc32fast::hash(out) != crc)
                {
                    return Err(FormatError::Checksum);
                }
            }
            Compression::Deflate => {
                let input = FileWindow {
                    file: &self.file,
                    at: entry.offset,
                    remaining: entry.compressed_length,
                };
                let mut input = input;
                reader.decoder.reset(false);
                let mut first = 0;
                let mut last = 0;
                let mut crc = crc32fast::Hasher::new();
                loop {
                    if first == last {
                        last = input.read(&mut reader.input).map_err(io_error)?;
                        first = 0;
                    }
                    let before_in = reader.decoder.total_in();
                    let before_out = reader.decoder.total_out();
                    let end = offset + count as u64;
                    let destination = if before_out < offset {
                        let discard =
                            (offset - before_out).min(reader.output.len() as u64) as usize;
                        &mut reader.output[..discard]
                    } else if before_out < end {
                        &mut out[(before_out - offset) as usize..]
                    } else {
                        &mut reader.output[..]
                    };
                    let status = reader
                        .decoder
                        .decompress(
                            &reader.input[first..last],
                            destination,
                            flate2::FlushDecompress::None,
                        )
                        .map_err(|_| FormatError::Compression)?;
                    let consumed = reader.decoder.total_in() - before_in;
                    let produced = reader.decoder.total_out() - before_out;
                    crc.update(&destination[..produced as usize]);
                    first += consumed as usize;
                    if reader.decoder.total_out() > entry.length {
                        return Err(FormatError::Compression);
                    }
                    if status == flate2::Status::StreamEnd {
                        if reader.decoder.total_in() != entry.compressed_length
                            || reader.decoder.total_out() != entry.length
                        {
                            return Err(FormatError::Compression);
                        }
                        if entry
                            .crc32
                            .is_some_and(|expected| crc.finalize() != expected)
                        {
                            return Err(FormatError::Checksum);
                        }
                        break;
                    }
                    if consumed == 0 && produced == 0 {
                        return Err(FormatError::Compression);
                    }
                }
            }
        }
        Ok(count)
    }

    /// Stored members support range reads. Deflated assets load as whole members;
    /// audio/rendering retain decoded data, rather than reopening compressed files.
    pub fn read_at(
        &self,
        index: usize,
        offset: u64,
        destination: &mut [u8],
    ) -> Result<usize, FormatError> {
        let entry = self.entries.get(index).ok_or(FormatError::InvalidRange)?;
        if offset == 0 && destination.len() as u64 >= entry.length {
            return self.read_into(index, destination);
        }
        if entry.compression != Compression::Stored {
            return Err(FormatError::InvalidRange);
        }
        let remaining = entry
            .length
            .checked_sub(offset)
            .ok_or(FormatError::InvalidRange)?;
        let count = destination
            .len()
            .min(usize::try_from(remaining).unwrap_or(usize::MAX));
        exact(&self.file, entry.offset + offset, &mut destination[..count])?;
        Ok(count)
    }
}

struct FileWindow<'a> {
    file: &'a File,
    at: u64,
    remaining: u64,
}
impl Read for FileWindow<'_> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let count = out
            .len()
            .min(usize::try_from(self.remaining).unwrap_or(usize::MAX));
        let read = read_at(self.file, &mut out[..count], self.at)?;
        self.at += read as u64;
        self.remaining -= read as u64;
        Ok(read)
    }
}
