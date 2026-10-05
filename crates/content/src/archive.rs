//! Game-content archives: PACK readers, ZIP-family readers, and loose files.
//!
//! Donors: `src/content/archive/types.ts`, `source.ts`, `pak.ts`, `zip.ts`,
//! `loose.ts`, and `index.ts`. The donor is async over Node file handles;
//! this port is synchronous over `std::fs`. `ArchiveHandle` and
//! `MemoryArchiveHandle` collapse into the single synchronous
//! [`OpenArchive`]; entry lookup, directory parsing, and range checks keep
//! donor semantics and messages.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fs::File;
#[cfg(not(unix))]
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use flate2::Decompress;
use flate2::FlushDecompress;
use qa_core::binary::{BinaryError, BinaryReader};
use thiserror::Error;

use crate::contract::{ArchiveFormat, MAX_SAFE_INTEGER};

/// Archive failure (donor `ArchiveError`, `RangeError`, and I/O throws).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ArchiveError {
    /// Offset-addressed container failure (`source:offset: message`).
    #[error("{input}:{offset}: {message}")]
    Archive {
        /// Failing source label.
        input: String,
        /// Byte offset of the failure.
        offset: u64,
        /// What went wrong.
        message: String,
    },
    /// Archive member path is unsafe (`normalizeEntryPath`).
    #[error("Unsafe archive member path: {0:?}")]
    UnsafePath(String),
    /// Invalid archive use (donor `RangeError` throws).
    #[error("{0}")]
    Invalid(String),
    /// Filesystem access failed.
    #[error("Archive I/O error for {path}: {message}")]
    Io {
        /// Requested path.
        path: String,
        /// Underlying error.
        message: String,
    },
}

impl ArchiveError {
    /// Build an offset-addressed container failure.
    #[must_use]
    pub fn archive(source: &str, offset: u64, message: impl Into<String>) -> Self {
        Self::Archive {
            input: source.to_string(),
            offset,
            message: message.into(),
        }
    }
}

impl From<BinaryError> for ArchiveError {
    fn from(error: BinaryError) -> Self {
        Self::Archive {
            input: error.input,
            offset: error.offset as u64,
            message: error.message,
        }
    }
}

/// Reject a byte range outside a `size`-byte source (`checkRange`).
pub fn check_range(source: &str, size: u64, offset: u64, length: u64) -> Result<(), ArchiveError> {
    if length > size || offset > size - length {
        return Err(ArchiveError::archive(
            source,
            offset,
            format!("range of {length} bytes exceeds {size}-byte source"),
        ));
    }
    Ok(())
}

/// Normalize an archive member path (`normalizeEntryPath`).
///
/// Separators become `/`, internal `.` segments and escapes are rejected,
/// case is retained, and a trailing `/` marks a directory.
pub fn normalize_entry_path(path: &str) -> Result<String, ArchiveError> {
    let separated = path.replace('\\', "/");
    let trailing = separated.ends_with('/');
    let candidate = if trailing {
        &separated[..separated.len() - 1]
    } else {
        separated.as_str()
    };
    if candidate.is_empty()
        || candidate.contains('\0')
        || candidate.contains(':')
        || candidate.split('/').any(|part| part.is_empty() || part == ".")
    {
        return Err(ArchiveError::UnsafePath(path.to_string()));
    }
    let mut parts: Vec<&str> = Vec::new();
    for part in candidate.split('/') {
        if part == ".." {
            if parts.is_empty() {
                return Err(ArchiveError::UnsafePath(path.to_string()));
            }
            parts.pop();
        } else {
            parts.push(part);
        }
    }
    if parts.is_empty() {
        return Err(ArchiveError::UnsafePath(path.to_string()));
    }
    let mut normalized = parts.join("/");
    if trailing {
        normalized.push('/');
    }
    Ok(normalized)
}

/// How member paths compare during lookup (`ArchivePathComparison`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArchivePathComparison {
    /// Byte-exact match after normalization.
    Exact,
    /// ASCII letters compare case-insensitively.
    AsciiInsensitive,
    /// Full Unicode case-insensitive match.
    CaseInsensitive,
}

/// Fold a member path for comparison (`compareEntryPath`).
pub fn compare_entry_path(path: &str, comparison: ArchivePathComparison) -> Result<String, ArchiveError> {
    let normalized = normalize_entry_path(path)?;
    match comparison {
        ArchivePathComparison::Exact => Ok(normalized),
        ArchivePathComparison::CaseInsensitive => Ok(normalized.to_lowercase()),
        ArchivePathComparison::AsciiInsensitive => Ok(normalized
            .chars()
            .map(|character| {
                if character.is_ascii_uppercase() {
                    character.to_ascii_lowercase()
                } else {
                    character
                }
            })
            .collect()),
    }
}

/// PACK directory member (`PakEntry`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PakEntry {
    /// Directory ordinal.
    pub ordinal: usize,
    /// Normalized member path.
    pub path: String,
    /// Raw stored member path.
    pub raw_path: String,
    /// Uncompressed byte length.
    pub byte_length: u64,
    /// Stored byte length (always equal for PACK).
    pub compressed_size: u64,
    /// Whether the member is a directory.
    pub is_directory: bool,
    /// Payload offset in the container.
    pub data_offset: u64,
}

/// ZIP-family compression method (donor codes `0` and `8`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ZipCompression {
    /// Stored without compression (code `0`).
    Store,
    /// Raw DEFLATE stream (code `8`).
    Deflate,
}

impl ZipCompression {
    /// Donor numeric method code.
    #[must_use]
    pub fn code(self) -> u16 {
        match self {
            ZipCompression::Store => 0,
            ZipCompression::Deflate => 8,
        }
    }
}

/// ZIP-family central-directory member (`ZipEntry`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipEntry {
    /// Directory ordinal.
    pub ordinal: usize,
    /// Normalized member path.
    pub path: String,
    /// Raw stored member path.
    pub raw_path: String,
    /// Uncompressed byte length.
    pub byte_length: u64,
    /// Stored byte length.
    pub compressed_size: u64,
    /// Whether the member is a directory.
    pub is_directory: bool,
    /// Container format (never PACK).
    pub format: ArchiveFormat,
    /// Compression method.
    pub compression_method: ZipCompression,
    /// General-purpose flag bits.
    pub flags: u16,
    /// Stored CRC-32.
    pub crc32: u32,
    /// Local header offset, adjusted for any self-extractor prefix.
    pub local_header_offset: u64,
}

/// Archive member (`ArchiveEntry`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchiveEntry {
    /// PACK member.
    Pak(PakEntry),
    /// ZIP-family member.
    Zip(ZipEntry),
}

impl ArchiveEntry {
    /// Directory ordinal.
    #[must_use]
    pub fn ordinal(&self) -> usize {
        match self {
            ArchiveEntry::Pak(entry) => entry.ordinal,
            ArchiveEntry::Zip(entry) => entry.ordinal,
        }
    }

    /// Normalized member path.
    #[must_use]
    pub fn path(&self) -> &str {
        match self {
            ArchiveEntry::Pak(entry) => entry.path.as_str(),
            ArchiveEntry::Zip(entry) => entry.path.as_str(),
        }
    }

    /// Raw stored member path.
    #[must_use]
    pub fn raw_path(&self) -> &str {
        match self {
            ArchiveEntry::Pak(entry) => entry.raw_path.as_str(),
            ArchiveEntry::Zip(entry) => entry.raw_path.as_str(),
        }
    }

    /// Uncompressed byte length.
    #[must_use]
    pub fn byte_length(&self) -> u64 {
        match self {
            ArchiveEntry::Pak(entry) => entry.byte_length,
            ArchiveEntry::Zip(entry) => entry.byte_length,
        }
    }

    /// Stored byte length.
    #[must_use]
    pub fn compressed_size(&self) -> u64 {
        match self {
            ArchiveEntry::Pak(entry) => entry.compressed_size,
            ArchiveEntry::Zip(entry) => entry.compressed_size,
        }
    }

    /// Whether the member is a directory.
    #[must_use]
    pub fn is_directory(&self) -> bool {
        match self {
            ArchiveEntry::Pak(entry) => entry.is_directory,
            ArchiveEntry::Zip(entry) => entry.is_directory,
        }
    }

    /// Container format.
    #[must_use]
    pub fn format(&self) -> ArchiveFormat {
        match self {
            ArchiveEntry::Pak(_) => ArchiveFormat::Pak,
            ArchiveEntry::Zip(entry) => entry.format,
        }
    }
}

/// Entry selector: directory ordinal or a previously returned entry.
#[derive(Debug, Clone, Copy)]
pub enum EntryRef<'a> {
    /// Directory ordinal.
    Ordinal(usize),
    /// Previously returned entry (verified against this archive).
    Entry(&'a ArchiveEntry),
}

/// In-memory retained bytes (`MemorySource`).
///
/// The input is copied so later caller mutations cannot replace retained
/// entry bytes.
#[derive(Debug)]
pub struct MemorySource {
    bytes: Vec<u8>,
    source: String,
    closed: Cell<bool>,
}

impl MemorySource {
    /// Retain a copy of `bytes` under `source`.
    #[must_use]
    pub fn new(bytes: &[u8], source: &str) -> Self {
        Self {
            bytes: bytes.to_vec(),
            source: source.to_string(),
            closed: Cell::new(false),
        }
    }

    /// Source label for diagnostics.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Retained byte length.
    #[must_use]
    pub fn byte_length(&self) -> u64 {
        self.bytes.len() as u64
    }

    /// Whether the source is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed.get()
    }

    /// Read a checked range.
    pub fn read(&self, offset: u64, length: u64) -> Result<Vec<u8>, ArchiveError> {
        if self.closed.get() {
            return Err(ArchiveError::archive(&self.source, offset, "archive is closed"));
        }
        check_range(&self.source, self.byte_length(), offset, length)?;
        let offset = offset as usize;
        let length = length as usize;
        Ok(self.bytes[offset..offset + length].to_vec())
    }

    /// Release the retained bytes.
    pub fn close(&self) {
        self.closed.set(true);
    }
}

/// Filesystem identity of a retained file: the stat tuple that detects
/// replacement, truncation, or rewrite without reading any bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileIdentity {
    /// Filesystem device number (zero off unix).
    pub device: u64,
    /// Inode number (zero off unix).
    pub inode: u64,
    /// Byte length.
    pub size: u64,
    /// Last-modification time, seconds and nanoseconds.
    pub modified_secs: i64,
    /// Last-modification time, nanosecond part.
    pub modified_nanos: i64,
    /// Last-status-change time, seconds and nanoseconds (zero off unix).
    pub changed_secs: i64,
    /// Last-status-change time, nanosecond part.
    pub changed_nanos: i64,
}

#[cfg(unix)]
#[must_use]
fn file_identity(meta: &std::fs::Metadata) -> FileIdentity {
    use std::os::unix::fs::MetadataExt;
    FileIdentity {
        device: meta.dev(),
        inode: meta.ino(),
        size: meta.len(),
        modified_secs: meta.mtime(),
        modified_nanos: meta.mtime_nsec(),
        changed_secs: meta.ctime(),
        changed_nanos: meta.ctime_nsec(),
    }
}

#[cfg(not(unix))]
#[must_use]
fn file_identity(meta: &std::fs::Metadata) -> FileIdentity {
    let (modified_secs, modified_nanos) = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|elapsed| (elapsed.as_secs() as i64, elapsed.subsec_nanos() as i64))
        .unwrap_or((0, 0));
    FileIdentity {
        device: 0,
        inode: 0,
        size: meta.len(),
        modified_secs,
        modified_nanos,
        changed_secs: 0,
        changed_nanos: 0,
    }
}

/// Retained file descriptor (`FileSource`).
///
/// A retained descriptor keeps path replacements from changing an open
/// archive. Size and timestamps are recorded at open and checked by
/// [`FileSource::verify_identity`] at mount and map-load boundaries;
/// reads themselves do no metadata checks.
#[derive(Debug)]
pub struct FileSource {
    source: String,
    byte_length: u64,
    file: RefCell<File>,
    identity: FileIdentity,
    closed: Cell<bool>,
}

fn io_error(path: &Path, error: std::io::Error) -> ArchiveError {
    ArchiveError::Io {
        path: path.to_string_lossy().into_owned(),
        message: error.to_string(),
    }
}

impl FileSource {
    /// Retain `path`, recording its size and timestamps.
    pub fn new(path: &Path) -> Result<Self, ArchiveError> {
        let source = path.to_string_lossy().into_owned();
        let file = File::open(path).map_err(|error| io_error(path, error))?;
        let meta = file.metadata().map_err(|error| io_error(path, error))?;
        let byte_length = meta.len();
        if !meta.is_file() || byte_length > MAX_SAFE_INTEGER {
            return Err(ArchiveError::archive(
                &source,
                0,
                "expected a regular file with a safe byte length",
            ));
        }
        let identity = file_identity(&meta);
        Ok(Self {
            source,
            byte_length,
            file: RefCell::new(file),
            identity,
            closed: Cell::new(false),
        })
    }

    /// Source label for diagnostics.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Byte length recorded when the source was opened.
    #[must_use]
    pub fn byte_length(&self) -> u64 {
        self.byte_length
    }

    /// Whether the source is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed.get()
    }

    /// Filesystem identity recorded when the source was opened.
    #[must_use]
    pub fn identity(&self) -> FileIdentity {
        self.identity
    }

    /// Check the retained descriptor against its recorded identity.
    ///
    /// Callers run this at mount and map-load boundaries instead of on
    /// every read.
    pub fn verify_identity(&self) -> Result<(), ArchiveError> {
        if self.closed.get() {
            return Err(ArchiveError::archive(&self.source, 0, "archive is closed"));
        }
        let meta = self.file.borrow().metadata().map_err(|error| ArchiveError::Io {
            path: self.source.clone(),
            message: error.to_string(),
        })?;
        if file_identity(&meta) != self.identity {
            return Err(ArchiveError::archive(
                &self.source,
                0,
                "source changed after it was opened",
            ));
        }
        Ok(())
    }

    #[cfg(unix)]
    fn read_into(&self, offset: u64, bytes: &mut [u8]) -> Result<(), ArchiveError> {
        use std::os::unix::fs::FileExt;
        let file = self.file.borrow();
        let mut total = 0;
        while total < bytes.len() {
            let count = file
                .read_at(&mut bytes[total..], offset + total as u64)
                .map_err(|error| ArchiveError::Io {
                    path: self.source.clone(),
                    message: error.to_string(),
                })?;
            if count == 0 {
                return Err(ArchiveError::archive(
                    &self.source,
                    offset,
                    format!("short read: expected {}, got {total}", bytes.len()),
                ));
            }
            total += count;
        }
        Ok(())
    }

    #[cfg(not(unix))]
    fn read_into(&self, offset: u64, bytes: &mut [u8]) -> Result<(), ArchiveError> {
        let mut file = self.file.borrow_mut();
        file.seek(SeekFrom::Start(offset)).map_err(|error| ArchiveError::Io {
            path: self.source.clone(),
            message: error.to_string(),
        })?;
        let mut total = 0;
        while total < bytes.len() {
            let count = file.read(&mut bytes[total..]).map_err(|error| ArchiveError::Io {
                path: self.source.clone(),
                message: error.to_string(),
            })?;
            if count == 0 {
                return Err(ArchiveError::archive(
                    &self.source,
                    offset,
                    format!("short read: expected {}, got {total}", bytes.len()),
                ));
            }
            total += count;
        }
        Ok(())
    }

    /// Read a checked range through the retained descriptor.
    ///
    /// The read is positioned (no shared file offset) and performs no
    /// metadata checks; use [`FileSource::verify_identity`] at boundaries.
    pub fn read(&self, offset: u64, length: u64) -> Result<Vec<u8>, ArchiveError> {
        if self.closed.get() {
            return Err(ArchiveError::archive(&self.source, offset, "archive is closed"));
        }
        check_range(&self.source, self.byte_length, offset, length)?;
        let length_usize = usize::try_from(length)
            .map_err(|_| ArchiveError::archive(&self.source, offset, "range exceeds addressable memory"))?;
        let mut bytes = vec![0u8; length_usize];
        self.read_into(offset, &mut bytes)?;
        Ok(bytes)
    }

    /// Release the retained descriptor.
    pub fn close(&self) {
        self.closed.set(true);
    }
}

/// Retained archive bytes (`ArchiveSource`).
#[derive(Debug)]
pub enum ArchiveSource {
    /// In-memory bytes.
    Memory(MemorySource),
    /// Retained file descriptor.
    File(FileSource),
}

impl ArchiveSource {
    /// Source label for diagnostics.
    #[must_use]
    pub fn source(&self) -> &str {
        match self {
            ArchiveSource::Memory(source) => source.source(),
            ArchiveSource::File(source) => source.source(),
        }
    }

    /// Source byte length.
    #[must_use]
    pub fn byte_length(&self) -> u64 {
        match self {
            ArchiveSource::Memory(source) => source.byte_length(),
            ArchiveSource::File(source) => source.byte_length(),
        }
    }

    /// Whether the source is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        match self {
            ArchiveSource::Memory(source) => source.is_closed(),
            ArchiveSource::File(source) => source.is_closed(),
        }
    }

    /// Read a checked range.
    pub fn read(&self, offset: u64, length: u64) -> Result<Vec<u8>, ArchiveError> {
        match self {
            ArchiveSource::Memory(source) => source.read(offset, length),
            ArchiveSource::File(source) => source.read(offset, length),
        }
    }

    /// Check a file source against its recorded identity.
    ///
    /// Memory sources are immutable and always pass. Callers run this at
    /// mount and map-load boundaries instead of on every read.
    pub fn verify_identity(&self) -> Result<(), ArchiveError> {
        match self {
            ArchiveSource::Memory(_) => Ok(()),
            ArchiveSource::File(source) => source.verify_identity(),
        }
    }

    /// Release the source.
    pub fn close(&self) {
        match self {
            ArchiveSource::Memory(source) => source.close(),
            ArchiveSource::File(source) => source.close(),
        }
    }
}

/// PACK directory range (`readPakHeader` result).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PakDirectory {
    /// Directory offset.
    pub offset: u64,
    /// Directory byte length.
    pub byte_length: u64,
}

fn range_from_i32(source: &str, file_size: u64, offset: i32, length: i32) -> Result<(u64, u64), ArchiveError> {
    match (u64::try_from(offset), u64::try_from(length)) {
        (Ok(offset), Ok(length)) => Ok((offset, length)),
        // Donor offsets are signed; negative container ranges saturate to zero here.
        _ => Err(ArchiveError::archive(
            source,
            0,
            format!("range of {length} bytes exceeds {file_size}-byte source"),
        )),
    }
}

/// Parse the 12-byte PACK header (`readPakHeader`).
///
/// Adapted from quake-1-re-ts common.ts and quake-2-re-ts qcommon/files.ts.
/// id Software PACK header and ordered 64-byte directory. GPL-2.0-or-later.
pub fn read_pak_header(bytes: &[u8], source: &str, file_size: u64) -> Result<PakDirectory, ArchiveError> {
    let mut reader = BinaryReader::new(bytes, source);
    reader.expect_magic("PACK")?;
    let offset = reader.i32()?;
    let byte_length = reader.i32()?;
    let (offset, byte_length) = range_from_i32(source, file_size, offset, byte_length)?;
    check_range(source, file_size, offset, byte_length)?;
    if offset < 12 || byte_length % 64 != 0 {
        return Err(ArchiveError::archive(source, offset, "invalid PACK directory range"));
    }
    Ok(PakDirectory { offset, byte_length })
}

/// Parse the ordered 64-byte PACK directory (`readPakDirectory`).
pub fn read_pak_directory(bytes: &[u8], source: &str, file_size: u64) -> Result<Vec<PakEntry>, ArchiveError> {
    let mut reader = BinaryReader::new(bytes, source);
    let mut entries = Vec::new();
    while reader.remaining() > 0 {
        let raw_path = reader.fixed_byte_string(56)?;
        let data_offset = reader.i32()?;
        let byte_length = reader.i32()?;
        let (data_offset, byte_length) = range_from_i32(source, file_size, data_offset, byte_length)?;
        check_range(source, file_size, data_offset, byte_length)?;
        let path = normalize_entry_path(&raw_path)?;
        entries.push(PakEntry {
            ordinal: entries.len(),
            path: path.clone(),
            raw_path,
            byte_length,
            compressed_size: byte_length,
            is_directory: path.ends_with('/'),
            data_offset,
        });
    }
    Ok(entries)
}

/// Tail window holding any end-of-central-directory record (`ZIP_TAIL_LENGTH`).
pub const ZIP_TAIL_LENGTH: u64 = 22 + 0xffff;

/// ZIP central-directory range (`ZipDirectory`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZipDirectory {
    /// Central-directory offset, adjusted for any self-extractor prefix.
    pub offset: u64,
    /// Central-directory byte length.
    pub byte_length: u64,
    /// Directory entry count.
    pub entry_count: usize,
    /// Bytes before the archive proper (self-extractor prefix).
    pub prefix_length: u64,
}

#[must_use]
fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

#[must_use]
fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]])
}

/// Map Latin-1 bytes to the same code points (donor `byteString`).
#[must_use]
fn latin1_string(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| char::from(*byte)).collect()
}

/// Locate the end-of-central-directory record (`readZipTail`).
///
/// Central/local header validation adapted from quake-3-ts/src/assets/pk3.ts.
/// STORE/DEFLATE extraction follows quake-1-re-ts/src/lib/zipfile.ts.
/// GPL-2.0-or-later.
pub fn read_zip_tail(tail: &[u8], source: &str, file_size: u64) -> Result<ZipDirectory, ArchiveError> {
    let tail_length = tail.len() as u64;
    if tail.len() >= 22 {
        for offset in (0..=tail.len() - 22).rev() {
            if u32_at(tail, offset) != 0x06054b50 {
                continue;
            }
            if offset + 22 + usize::from(u16_at(tail, offset + 20)) != tail.len() {
                continue;
            }
            let disk = u16_at(tail, offset + 4);
            let central_disk = u16_at(tail, offset + 6);
            let disk_entries = u16_at(tail, offset + 8);
            let entry_count = u16_at(tail, offset + 10);
            let byte_length = u64::from(u32_at(tail, offset + 12));
            let relative_offset = u64::from(u32_at(tail, offset + 16));
            let end_offset = file_size - tail_length + offset as u64;
            if disk != 0 || central_disk != 0 || disk_entries != entry_count {
                return Err(ArchiveError::archive(
                    source,
                    end_offset,
                    "multi-disk ZIP is unsupported",
                ));
            }
            if entry_count == 0xffff || byte_length == 0xffffffff || relative_offset == 0xffffffff {
                return Err(ArchiveError::archive(source, end_offset, "ZIP64 is unsupported"));
            }
            check_range(source, end_offset, relative_offset, byte_length)?;
            let prefix_length = end_offset - relative_offset - byte_length;
            return Ok(ZipDirectory {
                offset: relative_offset + prefix_length,
                byte_length,
                entry_count: usize::from(entry_count),
                prefix_length,
            });
        }
    }
    Err(ArchiveError::archive(
        source,
        file_size - tail_length,
        "ZIP end-of-central-directory record not found",
    ))
}

/// Parse the central directory (`readZipDirectory`).
pub fn read_zip_directory(
    bytes: &[u8],
    source: &str,
    directory: &ZipDirectory,
    format: ArchiveFormat,
) -> Result<Vec<ZipEntry>, ArchiveError> {
    if format == ArchiveFormat::Pak {
        return Err(ArchiveError::archive(
            source,
            directory.offset,
            "ZIP directory requires a ZIP-family format",
        ));
    }
    let mut reader = BinaryReader::new(bytes, source);
    let mut entries = Vec::new();
    for ordinal in 0..directory.entry_count {
        let offset = reader.offset();
        let base = directory.offset + offset as u64;
        let header = reader
            .view(offset, 46)
            .map_err(|_| ArchiveError::archive(source, base, "truncated central directory entry"))?;
        if u32_at(header, 0) != 0x02014b50 {
            return Err(ArchiveError::archive(
                source,
                base,
                "invalid central directory signature",
            ));
        }
        let flags = u16_at(header, 8);
        let compression_method = u16_at(header, 10);
        let crc32 = u32_at(header, 16);
        let compressed_size = u64::from(u32_at(header, 20));
        let byte_length = u64::from(u32_at(header, 24));
        let name_length = usize::from(u16_at(header, 28));
        let extra_length = usize::from(u16_at(header, 30));
        let comment_length = usize::from(u16_at(header, 32));
        let disk = u16_at(header, 34);
        let local_offset = u64::from(u32_at(header, 42));
        if compressed_size == 0xffffffff || byte_length == 0xffffffff || local_offset == 0xffffffff {
            return Err(ArchiveError::archive(source, base, "ZIP64 entry is unsupported"));
        }
        if disk != 0 {
            return Err(ArchiveError::archive(source, base, "entry starts on another disk"));
        }
        if flags & 1 != 0 {
            return Err(ArchiveError::archive(source, base, "encrypted entry is unsupported"));
        }
        let compression_method = match compression_method {
            0 => ZipCompression::Store,
            8 => ZipCompression::Deflate,
            method => {
                return Err(ArchiveError::archive(
                    source,
                    base,
                    format!("unsupported compression method {method}"),
                ));
            }
        };
        if compression_method == ZipCompression::Store && compressed_size != byte_length {
            return Err(ArchiveError::archive(source, base, "stored entry sizes disagree"));
        }
        let local_header_offset = local_offset + directory.prefix_length;
        check_range(source, directory.offset, local_header_offset, 30)?;
        reader
            .skip(46)
            .map_err(|_| ArchiveError::archive(source, base, "truncated central directory entry"))?;
        let name = reader
            .bytes(name_length)
            .map_err(|_| ArchiveError::archive(source, base, "truncated central directory entry"))?;
        reader
            .skip(extra_length + comment_length)
            .map_err(|_| ArchiveError::archive(source, base, "truncated central directory entry"))?;
        let raw_path = latin1_string(&name);
        let path = normalize_entry_path(&raw_path)?;
        entries.push(ZipEntry {
            ordinal,
            path: path.clone(),
            raw_path,
            byte_length,
            compressed_size,
            is_directory: path.ends_with('/'),
            format,
            compression_method,
            flags,
            crc32,
            local_header_offset,
        });
    }
    if reader.remaining() != 0 {
        return Err(ArchiveError::archive(
            source,
            directory.offset + reader.offset() as u64,
            "central directory size disagrees with entry count",
        ));
    }
    Ok(entries)
}

/// ZIP local header variable-length sizes (`readZipLocalHeader` result).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZipLocalHeader {
    /// File-name length.
    pub name_length: usize,
    /// File-name plus extra-field length.
    pub byte_length: usize,
}

/// Validate a 30-byte local file header against its directory entry.
pub fn read_zip_local_header(bytes: &[u8], source: &str, entry: &ZipEntry) -> Result<ZipLocalHeader, ArchiveError> {
    let header = bytes
        .get(0..30)
        .ok_or_else(|| ArchiveError::archive(source, entry.local_header_offset, "truncated local file header"))?;
    if u32_at(header, 0) != 0x04034b50 {
        return Err(ArchiveError::archive(
            source,
            entry.local_header_offset,
            "invalid local file header signature",
        ));
    }
    let flags = u16_at(header, 6);
    let method = u16_at(header, 8);
    let crc32 = u32_at(header, 14);
    let compressed_size = u64::from(u32_at(header, 18));
    let byte_length = u64::from(u32_at(header, 22));
    // APPNOTE 4.4.4 reserves bit 15; legacy writers may leave it set in only one header.
    if (flags ^ entry.flags) & 0x7fff != 0 || method != entry.compression_method.code() {
        return Err(ArchiveError::archive(
            source,
            entry.local_header_offset,
            "local header disagrees with central directory",
        ));
    }
    let uses_descriptor = flags & 8 != 0;
    if (!uses_descriptor || crc32 != 0) && crc32 != entry.crc32
        || (!uses_descriptor || compressed_size != 0) && compressed_size != entry.compressed_size
        || (!uses_descriptor || byte_length != 0) && byte_length != entry.byte_length
    {
        return Err(ArchiveError::archive(
            source,
            entry.local_header_offset,
            "local sizes or CRC disagree with central directory",
        ));
    }
    let name_length = usize::from(u16_at(header, 26));
    let extra_length = usize::from(u16_at(header, 28));
    Ok(ZipLocalHeader {
        name_length,
        byte_length: name_length + extra_length,
    })
}

/// Resolve an entry payload offset, checking the local name (`zipDataOffset`).
pub fn zip_data_offset(
    variable: &[u8],
    name_length: usize,
    source: &str,
    central_offset: u64,
    entry: &ZipEntry,
) -> Result<u64, ArchiveError> {
    let name = variable.get(..name_length).ok_or_else(|| {
        ArchiveError::archive(
            source,
            entry.local_header_offset,
            "local entry name disagrees with central directory",
        )
    })?;
    if latin1_string(name) != entry.raw_path {
        return Err(ArchiveError::archive(
            source,
            entry.local_header_offset,
            "local entry name disagrees with central directory",
        ));
    }
    let data_offset = entry.local_header_offset + 30 + variable.len() as u64;
    check_range(source, central_offset, data_offset, entry.compressed_size)?;
    Ok(data_offset)
}

/// IEEE CRC-32 (donor `Bun.hash.crc32`).
#[must_use]
fn crc32(bytes: &[u8]) -> u32 {
    crc32fast::hash(bytes)
}

fn inflate_raw_capped(compressed: &[u8], cap: u64) -> Result<Vec<u8>, String> {
    let cap = usize::try_from(cap).map_err(|_| "output exceeds addressable memory".to_string())?;
    let mut output = vec![0u8; cap.saturating_add(1)];
    let mut decoder = Decompress::new(false);
    decoder
        .decompress(compressed, &mut output, FlushDecompress::Finish)
        .map_err(|error| error.to_string())?;
    let length = usize::try_from(decoder.total_out()).map_err(|_| "output exceeds addressable memory".to_string())?;
    if length > cap {
        return Err(format!("output exceeds {cap}-byte limit"));
    }
    output.truncate(length);
    Ok(output)
}

/// Decode an entry payload, checking size and CRC-32 (`decodeZipEntry`).
///
/// DEFLATE output is capped at the entry byte length (minimum one), matching
/// the donor `maxOutputLength` cap.
pub fn decode_zip_entry(
    compressed: &[u8],
    source: &str,
    data_offset: u64,
    entry: &ZipEntry,
) -> Result<Vec<u8>, ArchiveError> {
    let output = match entry.compression_method {
        ZipCompression::Store => compressed.to_vec(),
        ZipCompression::Deflate => inflate_raw_capped(compressed, entry.byte_length.max(1))
            .map_err(|message| ArchiveError::archive(source, data_offset, format!("DEFLATE failed: {message}")))?,
    };
    if output.len() as u64 != entry.byte_length {
        return Err(ArchiveError::archive(
            source,
            data_offset,
            format!("decoded size {} disagrees with {}", output.len(), entry.byte_length),
        ));
    }
    if crc32(&output) != entry.crc32 {
        return Err(ArchiveError::archive(source, data_offset, "entry CRC32 mismatch"));
    }
    Ok(output)
}

/// Open loose file confined to its root (`LooseEntryHandle`).
#[derive(Debug)]
pub struct LooseEntryHandle {
    /// Normalized member path.
    pub path: String,
    /// Canonical native source path.
    pub source: String,
    /// File byte length.
    pub byte_length: u64,
    storage: FileSource,
}

impl LooseEntryHandle {
    /// Read the whole file.
    pub fn read(&self) -> Result<Vec<u8>, ArchiveError> {
        self.storage.read(0, self.storage.byte_length())
    }

    /// Release the retained descriptor.
    pub fn close(&self) {
        self.storage.close();
    }
}

/// Open a loose file; native resolution stays confined to `root`.
pub fn open_loose_entry(root: &Path, member_path: &str) -> Result<LooseEntryHandle, ArchiveError> {
    let path = normalize_entry_path(member_path)?;
    if path.ends_with('/') {
        return Err(ArchiveError::Invalid("Expected a loose file path".to_string()));
    }
    let root = std::fs::canonicalize(root).map_err(|error| io_error(root, error))?;
    let joined = root.join(&path);
    let source = std::fs::canonicalize(&joined).map_err(|error| io_error(&joined, error))?;
    if source != root && !source.starts_with(&root) {
        return Err(ArchiveError::Invalid(format!(
            "Loose entry escapes root: {member_path}"
        )));
    }
    let storage = FileSource::new(&source)?;
    Ok(LooseEntryHandle {
        path,
        source: source.to_string_lossy().into_owned(),
        byte_length: storage.byte_length(),
        storage,
    })
}

/// Read a loose file, closing it before returning.
pub fn read_loose_entry(root: &Path, member_path: &str) -> Result<Vec<u8>, ArchiveError> {
    let entry = open_loose_entry(root, member_path)?;
    let bytes = entry.read()?;
    entry.close();
    Ok(bytes)
}

/// Open archive over a retained source (`OpenArchive`/`DecodedArchive`).
///
/// Payloads are read on demand; only the header/tail and directory are
/// parsed up front. Lookup indexes are built lazily per comparison.
#[derive(Debug)]
pub struct OpenArchive {
    /// Container format.
    pub format: ArchiveFormat,
    /// Source label for diagnostics.
    pub source: String,
    /// Source byte length.
    pub byte_length: u64,
    /// Directory entries in physical order.
    pub entries: Vec<ArchiveEntry>,
    storage: ArchiveSource,
    central_offset: u64,
    entry_indexes: RefCell<HashMap<ArchivePathComparison, HashMap<String, Vec<usize>>>>,
}

impl OpenArchive {
    /// Find entries by member path (cloned; empty when nothing matches).
    pub fn find_entries(
        &self,
        path: &str,
        comparison: ArchivePathComparison,
    ) -> Result<Vec<ArchiveEntry>, ArchiveError> {
        let key = compare_entry_path(path, comparison)?;
        if let Some(matches) = self
            .entry_indexes
            .borrow()
            .get(&comparison)
            .and_then(|index| index.get(&key))
        {
            return Ok(matches.iter().map(|&ordinal| self.entries[ordinal].clone()).collect());
        }
        let mut index: HashMap<String, Vec<usize>> = HashMap::new();
        for entry in &self.entries {
            index
                .entry(compare_entry_path(entry.path(), comparison)?)
                .or_default()
                .push(entry.ordinal());
        }
        let matches = index.get(&key).cloned().unwrap_or_default();
        self.entry_indexes.borrow_mut().insert(comparison, index);
        Ok(matches.iter().map(|&ordinal| self.entries[ordinal].clone()).collect())
    }

    fn select(&self, requested: EntryRef<'_>) -> Result<&ArchiveEntry, ArchiveError> {
        let ordinal = match requested {
            EntryRef::Ordinal(ordinal) => ordinal,
            EntryRef::Entry(entry) => entry.ordinal(),
        };
        let entry = self.entries.get(ordinal).ok_or_else(|| {
            ArchiveError::archive(
                &self.source,
                0,
                format!("entry {ordinal} does not belong to this archive"),
            )
        })?;
        // The donor compares object identity; value equality over the cloned
        // entry is the synchronous equivalent.
        if let EntryRef::Entry(requested) = requested {
            if requested != entry {
                return Err(ArchiveError::archive(
                    &self.source,
                    0,
                    format!("entry {ordinal} does not belong to this archive"),
                ));
            }
        }
        if entry.is_directory() {
            return Err(ArchiveError::archive(
                &self.source,
                0,
                format!("entry {ordinal} is a directory"),
            ));
        }
        Ok(entry)
    }

    /// Read and decode an entry payload.
    pub fn read_entry(&self, requested: EntryRef<'_>) -> Result<Vec<u8>, ArchiveError> {
        let entry = self.select(requested)?;
        match entry {
            ArchiveEntry::Pak(entry) => self.storage.read(entry.data_offset, entry.byte_length),
            ArchiveEntry::Zip(entry) => {
                let header =
                    read_zip_local_header(&self.storage.read(entry.local_header_offset, 30)?, &self.source, entry)?;
                let variable = self
                    .storage
                    .read(entry.local_header_offset + 30, header.byte_length as u64)?;
                let offset = zip_data_offset(&variable, header.name_length, &self.source, self.central_offset, entry)?;
                decode_zip_entry(
                    &self.storage.read(offset, entry.compressed_size)?,
                    &self.source,
                    offset,
                    entry,
                )
            }
        }
    }

    /// Check the retained storage against its recorded identity.
    ///
    /// Callers run this at mount and map-load boundaries instead of on
    /// every read.
    pub fn verify_storage(&self) -> Result<(), ArchiveError> {
        self.storage.verify_identity()
    }

    /// Drop lookup indexes and release the retained source.
    pub fn close(&self) {
        self.entry_indexes.borrow_mut().clear();
        self.storage.close();
    }
}

/// Read an entry payload through a shared handle (donor `readEntry`).
pub fn read_entry(archive: &OpenArchive, requested: EntryRef<'_>) -> Result<Vec<u8>, ArchiveError> {
    archive.read_entry(requested)
}

#[must_use]
fn detect_format(header: &[u8], source: &str) -> ArchiveFormat {
    if header.len() >= 4 && header[0] == b'P' && header[1] == b'A' && header[2] == b'C' && header[3] == b'K' {
        return ArchiveFormat::Pak;
    }
    let extension = Path::new(source)
        .extension()
        .map(|extension| extension.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "pk3" => ArchiveFormat::Pk3,
        "kpf" => ArchiveFormat::Kpf,
        "pak" => ArchiveFormat::Pak,
        _ => ArchiveFormat::Zip,
    }
}

fn parse_archive(
    storage: &ArchiveSource,
    format: Option<ArchiveFormat>,
) -> Result<(ArchiveFormat, Vec<ArchiveEntry>, u64), ArchiveError> {
    let source = storage.source().to_string();
    let byte_length = storage.byte_length();
    let header = storage.read(0, 12.min(byte_length))?;
    let selected = format.unwrap_or_else(|| detect_format(&header, &source));
    if selected == ArchiveFormat::Pak {
        let directory = read_pak_header(&header, &source, byte_length)?;
        let bytes = storage.read(directory.offset, directory.byte_length)?;
        let entries = read_pak_directory(&bytes, &source, byte_length)?;
        Ok((
            selected,
            entries.into_iter().map(ArchiveEntry::Pak).collect(),
            directory.offset,
        ))
    } else {
        let tail_length = ZIP_TAIL_LENGTH.min(byte_length);
        let tail = storage.read(byte_length - tail_length, tail_length)?;
        let directory = read_zip_tail(&tail, &source, byte_length)?;
        let bytes = storage.read(directory.offset, directory.byte_length)?;
        let entries = read_zip_directory(&bytes, &source, &directory, selected)?;
        Ok((
            selected,
            entries.into_iter().map(ArchiveEntry::Zip).collect(),
            directory.offset,
        ))
    }
}

/// Decode an in-memory archive, copying the input (`decodeArchive`).
pub fn decode_archive(bytes: &[u8], format: Option<ArchiveFormat>, source: &str) -> Result<OpenArchive, ArchiveError> {
    open_archive_source(ArchiveSource::Memory(MemorySource::new(bytes, source)), format)
}

/// Open a file archive, retaining its descriptor (`openArchive`).
pub fn open_archive(path: &Path, format: Option<ArchiveFormat>) -> Result<OpenArchive, ArchiveError> {
    open_archive_source(ArchiveSource::File(FileSource::new(path)?), format)
}

/// Open an archive over a retained source, taking ownership of it.
///
/// The source is closed when parsing fails, matching the donor.
pub fn open_archive_source(storage: ArchiveSource, format: Option<ArchiveFormat>) -> Result<OpenArchive, ArchiveError> {
    match parse_archive(&storage, format) {
        Ok((format, entries, central_offset)) => Ok(OpenArchive {
            source: storage.source().to_string(),
            byte_length: storage.byte_length(),
            format,
            entries,
            storage,
            central_offset,
            entry_indexes: RefCell::new(HashMap::new()),
        }),
        Err(error) => {
            storage.close();
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn entry_paths_normalize_and_compare() {
        assert_eq!(normalize_entry_path("maps\\base1.bsp").unwrap(), "maps/base1.bsp");
        assert_eq!(normalize_entry_path("a/b/../c").unwrap(), "a/c");
        assert_eq!(normalize_entry_path("docs/").unwrap(), "docs/");
        assert!(normalize_entry_path("").is_err());
        assert!(normalize_entry_path("a/./b").is_err());
        assert!(normalize_entry_path("../escape").is_err());
        assert!(normalize_entry_path("a/../../b").is_err());
        assert!(normalize_entry_path("a\0b").is_err());
        assert!(normalize_entry_path("c:/autoexec.cfg").is_err());
        assert!(normalize_entry_path("a//b").is_err());
        assert_eq!(
            compare_entry_path("Maps\\Base1.BSP", ArchivePathComparison::Exact).unwrap(),
            "Maps/Base1.BSP"
        );
        assert_eq!(
            compare_entry_path("Maps/Base1.BSP", ArchivePathComparison::AsciiInsensitive).unwrap(),
            "maps/base1.bsp"
        );
        assert_eq!(
            compare_entry_path("Maps/Base1.BSP", ArchivePathComparison::CaseInsensitive).unwrap(),
            "maps/base1.bsp"
        );
        // ASCII folding leaves non-ASCII letters untouched.
        assert_eq!(
            compare_entry_path("Ä/B", ArchivePathComparison::AsciiInsensitive).unwrap(),
            "Ä/b"
        );
        assert_eq!(
            compare_entry_path("Ä/B", ArchivePathComparison::CaseInsensitive).unwrap(),
            "ä/b"
        );
    }

    #[test]
    fn check_range_rejects_overruns() {
        assert!(check_range("s", 10, 0, 10).is_ok());
        assert!(check_range("s", 10, 10, 0).is_ok());
        let error = check_range("s", 10, 8, 3).unwrap_err();
        assert_eq!(error.to_string(), "s:8: range of 3 bytes exceeds 10-byte source");
        assert!(check_range("s", 10, 0, 11).is_err());
    }

    #[test]
    fn crc32_matches_reference() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32(b""), 0);
    }

    fn pak_fixture() -> Vec<u8> {
        let payload_a = b"hello pak";
        let payload_b = b"second file contents";
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"PACK");
        let dir_offset = 12 + payload_a.len() + payload_b.len();
        bytes.extend_from_slice(&(dir_offset as i32).to_le_bytes());
        bytes.extend_from_slice(&(3 * 64i32).to_le_bytes());
        bytes.extend_from_slice(payload_a);
        bytes.extend_from_slice(payload_b);
        for (name, offset, length) in [
            ("maps/base1.bsp", 12, payload_a.len()),
            ("SOUND/Shot.wav", 12 + payload_a.len(), payload_b.len()),
            ("docs/", 0, 0),
        ] {
            let mut raw = [0u8; 56];
            raw[..name.len()].copy_from_slice(name.as_bytes());
            bytes.extend_from_slice(&raw);
            bytes.extend_from_slice(&(offset as i32).to_le_bytes());
            bytes.extend_from_slice(&(length as i32).to_le_bytes());
        }
        bytes
    }

    #[test]
    fn pak_decodes_and_reads() {
        let bytes = pak_fixture();
        let archive = decode_archive(&bytes, None, "test.pak").unwrap();
        assert_eq!(archive.format, ArchiveFormat::Pak);
        assert_eq!(archive.entries.len(), 3);
        assert_eq!(archive.byte_length, bytes.len() as u64);
        let found = archive
            .find_entries("maps/base1.bsp", ArchivePathComparison::Exact)
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path(), "maps/base1.bsp");
        assert_eq!(found[0].compressed_size(), found[0].byte_length());
        assert_eq!(archive.read_entry(EntryRef::Ordinal(0)).unwrap(), b"hello pak".to_vec());
        assert_eq!(
            archive.read_entry(EntryRef::Entry(&found[0])).unwrap(),
            b"hello pak".to_vec()
        );
        // Case-insensitive lookup finds the mixed-case member.
        let folded = archive
            .find_entries("sound/shot.wav", ArchivePathComparison::CaseInsensitive)
            .unwrap();
        assert_eq!(folded.len(), 1);
        assert_eq!(folded[0].path(), "SOUND/Shot.wav");
        assert!(archive
            .find_entries("sound/shot.wav", ArchivePathComparison::Exact)
            .unwrap()
            .is_empty());
        // Directories cannot be read ("docs" does not match "docs/").
        assert!(archive
            .find_entries("docs", ArchivePathComparison::Exact)
            .unwrap()
            .is_empty());
        let directory = archive.find_entries("docs/", ArchivePathComparison::Exact).unwrap();
        assert_eq!(directory.len(), 1);
        assert!(directory[0].is_directory());
        assert!(archive.read_entry(EntryRef::Ordinal(2)).is_err());
        // Unknown ordinals and foreign entries are rejected.
        assert!(archive.read_entry(EntryRef::Ordinal(9)).is_err());
        let other = decode_archive(&bytes, None, "other.pak").unwrap();
        let foreign = other
            .find_entries("maps/base1.bsp", ArchivePathComparison::Exact)
            .unwrap();
        let mut tampered = foreign[0].clone();
        let ArchiveEntry::Pak(tampered_entry) = &mut tampered else {
            panic!("expected pak entry")
        };
        tampered_entry.path = "maps/other.bsp".to_string();
        assert!(archive.read_entry(EntryRef::Entry(&tampered)).is_err());
        // Reads fail after close, mirroring the closed retained source.
        archive.close();
        assert!(archive.read_entry(EntryRef::Ordinal(0)).is_err());
    }

    #[test]
    fn pak_rejects_bad_headers() {
        let mut bytes = pak_fixture();
        bytes[8..12].copy_from_slice(&10i32.to_le_bytes());
        assert!(decode_archive(&bytes, None, "bad.pak").is_err());
        assert!(decode_archive(b"NOPE", None, "bad.pak").is_err());
        assert!(decode_archive(b"", None, "bad.pak").is_err());
    }

    struct ZipPiece {
        name: &'static str,
        data: Vec<u8>,
        method: u16,
    }

    fn zip_fixture(pieces: &[ZipPiece]) -> Vec<u8> {
        let mut prepared = Vec::new();
        for piece in pieces {
            let payload = if piece.method == 8 {
                let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                encoder.write_all(&piece.data).unwrap();
                encoder.finish().unwrap()
            } else {
                piece.data.clone()
            };
            prepared.push((piece.name, piece.data.clone(), payload, piece.method));
        }
        let mut bytes = Vec::new();
        let mut locals = Vec::new();
        for (name, raw, payload, method) in &prepared {
            locals.push(bytes.len() as u32);
            bytes.extend_from_slice(&0x04034b50u32.to_le_bytes());
            bytes.extend_from_slice(&20u16.to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&method.to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&crc32(raw).to_le_bytes());
            bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&(raw.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(name.as_bytes());
            bytes.extend_from_slice(payload);
        }
        let central_offset = bytes.len() as u32;
        for ((name, raw, payload, method), local) in prepared.iter().zip(locals.iter()) {
            bytes.extend_from_slice(&0x02014b50u32.to_le_bytes());
            bytes.extend_from_slice(&0x031eu16.to_le_bytes());
            bytes.extend_from_slice(&20u16.to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&method.to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&crc32(raw).to_le_bytes());
            bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&(raw.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&0u32.to_le_bytes());
            bytes.extend_from_slice(&local.to_le_bytes());
            bytes.extend_from_slice(name.as_bytes());
        }
        let central_size = bytes.len() as u32 - central_offset;
        bytes.extend_from_slice(&0x06054b50u32.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&(prepared.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&(prepared.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&central_size.to_le_bytes());
        bytes.extend_from_slice(&central_offset.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes
    }

    #[test]
    fn zip_store_and_deflate_round_trip() {
        let bytes = zip_fixture(&[
            ZipPiece {
                name: "maps/q3dm1.bsp",
                data: b"bsp-bytes".to_vec(),
                method: 0,
            },
            ZipPiece {
                name: "scripts/common.shader",
                data: b"shader-bytes ".repeat(40),
                method: 8,
            },
            ZipPiece {
                name: "docs/",
                data: Vec::new(),
                method: 0,
            },
        ]);
        let archive = decode_archive(&bytes, None, "mod.pk3").unwrap();
        assert_eq!(archive.format, ArchiveFormat::Pk3);
        assert_eq!(archive.entries.len(), 3);
        assert_eq!(archive.read_entry(EntryRef::Ordinal(0)).unwrap(), b"bsp-bytes".to_vec());
        assert_eq!(
            archive.read_entry(EntryRef::Ordinal(1)).unwrap(),
            b"shader-bytes ".repeat(40)
        );
        // Extension detection falls back to ZIP; PACK magic always wins.
        let zipped = decode_archive(&bytes, None, "mod.zip").unwrap();
        assert_eq!(zipped.format, ArchiveFormat::Zip);
        let forced = decode_archive(&bytes, Some(ArchiveFormat::Kpf), "mod.bin").unwrap();
        assert_eq!(forced.format, ArchiveFormat::Kpf);
        assert_eq!(forced.entries.len(), 3);
    }

    #[test]
    fn zip_entry_decoding_validates_size_crc_and_cap() {
        let raw = b"payload-bytes ".repeat(20);
        let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&raw).unwrap();
        let compressed = encoder.finish().unwrap();
        let entry = ZipEntry {
            ordinal: 0,
            path: "a.bin".to_string(),
            raw_path: "a.bin".to_string(),
            byte_length: raw.len() as u64,
            compressed_size: compressed.len() as u64,
            is_directory: false,
            format: ArchiveFormat::Zip,
            compression_method: ZipCompression::Deflate,
            flags: 0,
            crc32: crc32(&raw),
            local_header_offset: 0,
        };
        assert_eq!(decode_zip_entry(&compressed, "s", 0, &entry).unwrap(), raw);
        // Output capped below the true length fails as a DEFLATE error.
        let capped = ZipEntry {
            byte_length: 4,
            ..entry.clone()
        };
        let error = decode_zip_entry(&compressed, "s", 7, &capped).unwrap_err();
        assert!(error.to_string().starts_with("s:7: DEFLATE failed"), "{error}");
        // Corrupt CRC fails after a successful inflate.
        let crc = ZipEntry {
            crc32: entry.crc32 ^ 1,
            ..entry.clone()
        };
        let error = decode_zip_entry(&compressed, "s", 0, &crc).unwrap_err();
        assert_eq!(error.to_string(), "s:0: entry CRC32 mismatch");
        // Stored entries compare sizes directly.
        let stored = ZipEntry {
            compression_method: ZipCompression::Store,
            byte_length: 3,
            compressed_size: 3,
            ..entry.clone()
        };
        assert!(decode_zip_entry(b"toolong", "s", 0, &stored).is_err());
    }

    #[test]
    fn zip_rejects_bad_tails_and_headers() {
        assert!(decode_archive(b"not a zip at all...........", None, "x.zip").is_err());
        let bytes = zip_fixture(&[ZipPiece {
            name: "a.txt",
            data: b"a".to_vec(),
            method: 0,
        }]);
        // Corrupt the central signature.
        let mut bad = bytes.clone();
        let eocd = bad.len() - 22;
        let central = u32::from_le_bytes([bad[eocd + 16], bad[eocd + 17], bad[eocd + 18], bad[eocd + 19]]) as usize;
        bad[central] ^= 0xff;
        assert!(decode_archive(&bad, None, "x.zip").is_err());
        // Multi-disk tail.
        let mut multidisk = bytes.clone();
        let eocd = multidisk.len() - 22;
        multidisk[eocd + 4] = 1;
        let error = decode_archive(&multidisk, None, "x.zip").unwrap_err();
        assert!(error.to_string().contains("multi-disk"), "{error}");
        // Local header disagreement is reported against the directory entry.
        let archive = decode_archive(&bytes, None, "x.zip").unwrap();
        let ArchiveEntry::Zip(entry) = &archive.entries[0] else {
            panic!("expected zip entry")
        };
        let mut local = archive.storage.read(entry.local_header_offset, 30).unwrap();
        local[8] = 8;
        let error = read_zip_local_header(&local, "x.zip", entry).unwrap_err();
        assert!(error.to_string().contains("local header disagrees"), "{error}");
    }

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("qa-archive-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn loose_entries_read_and_stay_confined() {
        let root = scratch_dir("loose");
        std::fs::create_dir_all(root.join("maps")).unwrap();
        std::fs::write(root.join("maps").join("a.bsp"), b"loose-bytes").unwrap();
        let entry = open_loose_entry(&root, "maps/a.bsp").unwrap();
        assert_eq!(entry.path, "maps/a.bsp");
        assert_eq!(entry.byte_length, 11);
        assert_eq!(entry.read().unwrap(), b"loose-bytes".to_vec());
        entry.close();
        assert_eq!(read_loose_entry(&root, "maps\\a.bsp").unwrap(), b"loose-bytes".to_vec());
        assert!(open_loose_entry(&root, "maps/").is_err());
        assert!(open_loose_entry(&root, "../escape").is_err());
        assert!(read_loose_entry(&root, "maps/missing.bsp").is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn file_sources_detect_changes_and_close() {
        let root = scratch_dir("source");
        let path = root.join("data.bin");
        std::fs::write(&path, b"version-one").unwrap();
        let source = FileSource::new(&path).unwrap();
        assert_eq!(source.read(0, 11).unwrap(), b"version-one".to_vec());
        assert!(source.verify_identity().is_ok());
        std::fs::write(&path, b"version-two-much-longer").unwrap();
        // Reads stay cheap; the boundary check reports the change.
        assert_eq!(source.read(0, 11).unwrap(), b"version-two".to_vec());
        let error = source.verify_identity().unwrap_err();
        assert!(
            error.to_string().contains("source changed after it was opened"),
            "{error}"
        );
        source.close();
        assert!(source.is_closed());
        assert!(source.read(0, 1).is_err());
        assert!(source.verify_identity().is_err());
        assert!(FileSource::new(&root).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn memory_sources_copy_and_close() {
        let mut bytes = b"copy-me".to_vec();
        let source = MemorySource::new(&bytes, "<test>");
        bytes[0] = b'X';
        assert_eq!(source.read(0, 7).unwrap(), b"copy-me".to_vec());
        assert!(source.read(0, 8).is_err());
        source.close();
        assert!(source.read(0, 1).is_err());
    }

    #[test]
    fn open_archive_reads_files_on_demand() {
        let root = scratch_dir("open");
        let path = root.join("q2.pak");
        std::fs::write(&path, pak_fixture()).unwrap();
        let archive = open_archive(&path, None).unwrap();
        assert_eq!(archive.format, ArchiveFormat::Pak);
        assert_eq!(
            archive.read_entry(EntryRef::Ordinal(1)).unwrap(),
            b"second file contents".to_vec()
        );
        // Explicit formats override detection; a wrong override fails to parse.
        assert!(open_archive(&path, Some(ArchiveFormat::Zip)).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
