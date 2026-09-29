//! Download windows and staged sinks ported from
//! `src/network/services/downloads.ts`.
//!
//! Download windows follow Quake III `sv_client.c`. Sinks stage into the
//! contained parent directory and publish with a hard link, which fails if
//! the destination exists, so downloads never replace installed assets. HTTP
//! bodies stream through [`download_body`]; the host owns TLS and fetching.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use thiserror::Error;

use crate::common::hash::Sha256;
use crate::common::session::ContentDigest;

/// Error for download failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DownloadError {
    /// File path must be relative and stay inside its storage directory.
    #[error("File path must be relative and stay inside its storage directory")]
    EscapesRoot,
    /// Download source is not a regular bounded file.
    #[error("Download source is not a regular bounded file")]
    NotRegularFile,
    /// Download source is closed.
    #[error("Download source is closed")]
    SourceClosed,
    /// Invalid download range.
    #[error("Invalid download range")]
    BadRange,
    /// Download changed size while open.
    #[error("Download changed size while open")]
    SizeChanged,
    /// Unexpected download EOF.
    #[error("Unexpected download EOF")]
    UnexpectedEof,
    /// Download sink is closed.
    #[error("Download sink is closed")]
    SinkClosed,
    /// Download inspection is in progress.
    #[error("Download inspection is in progress")]
    Inspecting,
    /// Invalid download size.
    #[error("Invalid download size")]
    BadSize,
    /// Invalid ranged download size.
    #[error("Invalid ranged download size")]
    BadRangedSize,
    /// Download spans must partition the complete file.
    #[error("Download spans must partition the complete file")]
    BadSpans,
    /// Ranged downloads require appendRange.
    #[error("Ranged downloads require appendRange")]
    RequiresRange,
    /// Sequential downloads require append.
    #[error("Sequential downloads require append")]
    RequiresAppend,
    /// Download exceeds expected size.
    #[error("Download exceeds expected size")]
    ExceedsSize,
    /// Download exceeds span size.
    #[error("Download exceeds span size")]
    ExceedsSpan,
    /// Unknown download span.
    #[error("Unknown download span")]
    UnknownSpan,
    /// Download write made no progress.
    #[error("Download write made no progress")]
    NoProgress,
    /// Ranged download is incomplete.
    #[error("Ranged download is incomplete")]
    Incomplete,
    /// Ranged download staged size changed.
    #[error("Ranged download staged size changed")]
    StagedSizeChanged,
    /// Download size differs from content identity.
    #[error("Download size differs from content identity")]
    SizeMismatch,
    /// Download checksum differs from content identity.
    #[error("Download checksum differs from content identity")]
    DigestMismatch,
    /// Download sink closed during inspection.
    #[error("Download sink closed during inspection")]
    ClosedDuringInspection,
    /// Download window is closed.
    #[error("Download window is closed")]
    WindowClosed,
    /// Invalid download window.
    #[error("Invalid download window")]
    BadWindow,
    /// Broken download acknowledgement.
    #[error("Broken download acknowledgement")]
    BadAcknowledgement,
    /// Missing download block.
    #[error("Missing download block")]
    MissingBlock,
    /// Download block was not retained.
    #[error("Download block was not retained")]
    BlockNotRetained,
    /// Downloads require HTTP or HTTPS.
    #[error("Downloads require HTTP or HTTPS")]
    BadScheme,
    /// Filesystem failure.
    #[error("Download filesystem failure: {0}")]
    Filesystem(String),
}

/// Validate a contained relative path (`containedFileParts`, re-exported as
/// `downloadPath`).
pub fn download_path(name: &str) -> Result<Vec<String>, DownloadError> {
    if name.is_empty() || name.contains('\\') || name.contains('\0') || name.contains(':') {
        return Err(DownloadError::EscapesRoot);
    }
    let parts: Vec<String> = name.split('/').map(str::to_owned).collect();
    if parts.iter().any(|part| part.is_empty() || part == "." || part == "..") {
        return Err(DownloadError::EscapesRoot);
    }
    Ok(parts)
}

/// Byte source for downloads (`DownloadSource`).
pub trait DownloadSource {
    /// Source length in bytes.
    fn byte_length(&self) -> u64;
    /// Read up to `max_bytes` at `offset`.
    fn read(&mut self, offset: u64, max_bytes: usize) -> Result<Vec<u8>, DownloadError>;
    /// Close the source.
    fn close(&mut self);
}

/// File-backed download source (`DownloadFile`).
pub struct DownloadFile {
    file: Option<File>,
    byte_length: u64,
}

impl DownloadFile {
    /// Open a contained file for download (`DownloadFile.open`).
    pub fn open(root: &Path, name: &str) -> Result<Self, DownloadError> {
        let parts = download_path(name)?;
        let mut path = root.to_path_buf();
        for part in &parts {
            path.push(part);
        }
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|error| DownloadError::Filesystem(error.to_string()))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(DownloadError::NotRegularFile);
        }
        let file = File::open(&path).map_err(|error| DownloadError::Filesystem(error.to_string()))?;
        Ok(Self {
            byte_length: metadata.len(),
            file: Some(file),
        })
    }
}

impl DownloadSource for DownloadFile {
    fn byte_length(&self) -> u64 {
        self.byte_length
    }

    fn read(&mut self, offset: u64, max_bytes: usize) -> Result<Vec<u8>, DownloadError> {
        let Some(file) = self.file.as_mut() else {
            return Err(DownloadError::SourceClosed);
        };
        if offset > self.byte_length {
            return Err(DownloadError::BadRange);
        }
        let length = file
            .metadata()
            .map_err(|error| DownloadError::Filesystem(error.to_string()))?
            .len();
        if length != self.byte_length {
            return Err(DownloadError::SizeChanged);
        }
        file.seek(SeekFrom::Start(offset))
            .map_err(|error| DownloadError::Filesystem(error.to_string()))?;
        let mut bytes = vec![0u8; (self.byte_length - offset).min(max_bytes as u64) as usize];
        file.read_exact(&mut bytes)
            .map_err(|_| DownloadError::UnexpectedEof)?;
        Ok(bytes)
    }

    fn close(&mut self) {
        self.file = None;
    }
}

/// In-memory download source for tests and generated content.
pub struct MemorySource {
    bytes: Vec<u8>,
    ended: bool,
}

impl MemorySource {
    /// Wrap bytes in a source.
    #[must_use]
    pub fn new(bytes: Vec<u8>) -> Self {
        Self { bytes, ended: false }
    }
}

impl DownloadSource for MemorySource {
    fn byte_length(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn read(&mut self, offset: u64, max_bytes: usize) -> Result<Vec<u8>, DownloadError> {
        if self.ended {
            return Err(DownloadError::SourceClosed);
        }
        if offset > self.bytes.len() as u64 {
            return Err(DownloadError::BadRange);
        }
        let end = (self.bytes.len() as u64).min(offset + max_bytes as u64) as usize;
        Ok(self.bytes[offset as usize..end].to_vec())
    }

    fn close(&mut self) {
        self.ended = true;
    }
}

/// Expected digest and length (`DownloadExpectation`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadExpectation {
    /// Expected SHA-256 digest.
    pub digest: ContentDigest,
    /// Expected length.
    pub byte_length: u64,
}

/// Protocol-completion expectation with only a maximum
/// (`ProtocolDownloadExpectation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolDownloadExpectation {
    /// Maximum bytes.
    pub maximum_bytes: u64,
}

/// Sink expectation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SinkExpectation {
    /// Content-identity expectation.
    Content(DownloadExpectation),
    /// Protocol-completion expectation.
    Protocol(ProtocolDownloadExpectation),
}

impl SinkExpectation {
    fn limit(&self) -> u64 {
        match self {
            Self::Content(expected) => expected.byte_length,
            Self::Protocol(expected) => expected.maximum_bytes,
        }
    }
}

/// Ranged span (`DownloadSpan`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DownloadSpan {
    /// Inclusive start.
    pub start: u64,
    /// Inclusive end.
    pub end: u64,
}

struct RangeSpan {
    end: u64,
    written: u64,
}

enum WriteMode {
    Sequential,
    Ranged {
        total: u64,
        spans: BTreeMap<u64, RangeSpan>,
    },
}

static SINK_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Staged download sink (`DownloadSink`).
pub struct DownloadSink {
    file: Option<File>,
    staged: PathBuf,
    target: PathBuf,
    expected: SinkExpectation,
    mode: WriteMode,
    count: u64,
    ended: bool,
    inspecting: bool,
}

impl DownloadSink {
    /// Create a sequential sink (`DownloadSink.create`).
    pub fn create(
        root: &Path,
        name: &str,
        expected: SinkExpectation,
    ) -> Result<Self, DownloadError> {
        Self::open(root, name, expected, WriteMode::Sequential)
    }

    /// Create a ranged sink (`DownloadSink.createRanged`).
    pub fn create_ranged(
        root: &Path,
        name: &str,
        expected: SinkExpectation,
        total: u64,
        spans: &[DownloadSpan],
    ) -> Result<Self, DownloadError> {
        let limit = expected.limit();
        if total > limit {
            return Err(DownloadError::BadRangedSize);
        }
        if let SinkExpectation::Content(expected) = &expected {
            if total != expected.byte_length {
                return Err(DownloadError::BadRangedSize);
            }
        }
        let mut ordered = spans.to_vec();
        ordered.sort_by_key(|span| span.start);
        let mut ranges = BTreeMap::new();
        let mut next = 0;
        for span in &ordered {
            if span.start != next || span.end < span.start || (total > 0 && span.end >= total) {
                return Err(DownloadError::BadSpans);
            }
            ranges.insert(span.start, RangeSpan {
                end: span.end,
                written: 0,
            });
            next = span.end + 1;
        }
        if next != total {
            return Err(DownloadError::BadSpans);
        }
        Self::open(root, name, expected, WriteMode::Ranged { total, spans: ranges })
    }

    fn open(
        root: &Path,
        name: &str,
        expected: SinkExpectation,
        mode: WriteMode,
    ) -> Result<Self, DownloadError> {
        let parts = download_path(name)?;
        let mut parent = root.to_path_buf();
        for part in &parts[..parts.len() - 1] {
            parent.push(part);
        }
        std::fs::create_dir_all(&parent).map_err(|error| DownloadError::Filesystem(error.to_string()))?;
        let leaf = parts[parts.len() - 1].clone();
        let sequence = SINK_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let staged = parent.join(format!(
            ".download-{}-{}",
            std::process::id(),
            sequence
        ));
        let target = parent.join(leaf);
        let file = OpenOptions::new()
            .write(true)
            .read(true)
            .create_new(true)
            .mode(0o600)
            .open(&staged)
            .map_err(|error| DownloadError::Filesystem(error.to_string()))?;
        Ok(Self {
            file: Some(file),
            staged,
            target,
            expected,
            mode,
            count: 0,
            ended: false,
            inspecting: false,
        })
    }

    /// Bytes written so far.
    #[must_use]
    pub fn byte_length(&self) -> u64 {
        self.count
    }

    fn file(&mut self) -> Result<&mut File, DownloadError> {
        if self.inspecting {
            return Err(DownloadError::Inspecting);
        }
        self.file.as_mut().ok_or(DownloadError::SinkClosed)
    }

    /// Append sequential bytes (`append`).
    pub fn append(&mut self, bytes: &[u8]) -> Result<(), DownloadError> {
        if !matches!(self.mode, WriteMode::Sequential) {
            return Err(DownloadError::RequiresRange);
        }
        if self.ended {
            return Err(DownloadError::SinkClosed);
        }
        if self.inspecting {
            return Err(DownloadError::Inspecting);
        }
        if self.count + bytes.len() as u64 > self.expected.limit() {
            return Err(DownloadError::ExceedsSize);
        }
        let file = self.file()?;
        file.write_all(bytes).map_err(|error| DownloadError::Filesystem(error.to_string()))?;
        self.count += bytes.len() as u64;
        Ok(())
    }

    /// Append ranged bytes (`appendRange`).
    pub fn append_range(&mut self, span_start: u64, bytes: &[u8]) -> Result<(), DownloadError> {
        let WriteMode::Ranged { spans, .. } = &mut self.mode else {
            return Err(DownloadError::RequiresAppend);
        };
        if self.ended {
            return Err(DownloadError::SinkClosed);
        }
        if self.inspecting {
            return Err(DownloadError::Inspecting);
        }
        let span = spans.get_mut(&span_start).ok_or(DownloadError::UnknownSpan)?;
        if bytes.len() as u64 > span.end - span_start + 1 - span.written {
            return Err(DownloadError::ExceedsSpan);
        }
        let file = self.file.as_mut().ok_or(DownloadError::SinkClosed)?;
        file.seek(SeekFrom::Start(span_start + span.written))
            .map_err(|error| DownloadError::Filesystem(error.to_string()))?;
        file.write_all(bytes).map_err(|error| DownloadError::Filesystem(error.to_string()))?;
        span.written += bytes.len() as u64;
        self.count += bytes.len() as u64;
        Ok(())
    }

    fn require_complete_ranges(&mut self) -> Result<(), DownloadError> {
        let WriteMode::Ranged { total, spans } = &self.mode else {
            return Ok(());
        };
        if self.count != *total
            || spans.iter().any(|(start, span)| span.written != span.end - start + 1)
        {
            return Err(DownloadError::Incomplete);
        }
        let length = self
            .file
            .as_mut()
            .ok_or(DownloadError::SinkClosed)?
            .metadata()
            .map_err(|error| DownloadError::Filesystem(error.to_string()))?
            .len();
        if length != *total {
            return Err(DownloadError::StagedSizeChanged);
        }
        Ok(())
    }

    fn digest(&mut self) -> Result<ContentDigest, DownloadError> {
        let mut hasher = Sha256::new();
        let file = self.file.as_mut().ok_or(DownloadError::SinkClosed)?;
        file.seek(SeekFrom::Start(0))
            .map_err(|error| DownloadError::Filesystem(error.to_string()))?;
        let mut buffer = [0u8; 65536];
        loop {
            let count = file
                .read(&mut buffer)
                .map_err(|error| DownloadError::Filesystem(error.to_string()))?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
        Ok(ContentDigest::new(&crate::common::hash::hex_lower(&hasher.finish()))
            .map_err(|_| DownloadError::DigestMismatch)?)
    }

    /// Verify and publish the download (`finish`).
    pub fn finish(&mut self) -> Result<ContentDigest, DownloadError> {
        if self.ended {
            return Err(DownloadError::SinkClosed);
        }
        if self.inspecting {
            return Err(DownloadError::Inspecting);
        }
        let result = self.finish_inner();
        self.close();
        result
    }

    fn finish_inner(&mut self) -> Result<ContentDigest, DownloadError> {
        self.require_complete_ranges()?;
        if let SinkExpectation::Content(expected) = &self.expected {
            if self.count != expected.byte_length {
                return Err(DownloadError::SizeMismatch);
            }
        }
        // Flush staged bytes before hashing.
        if let Some(file) = self.file.as_mut() {
            file.flush().map_err(|error| DownloadError::Filesystem(error.to_string()))?;
        }
        let digest = self.digest()?;
        if let SinkExpectation::Content(expected) = &self.expected {
            if digest != expected.digest {
                return Err(DownloadError::DigestMismatch);
            }
        }
        // link fails if the destination exists, avoiding replacement of
        // installed game assets.
        std::fs::hard_link(&self.staged, &self.target)
            .map_err(|error| DownloadError::Filesystem(error.to_string()))?;
        Ok(digest)
    }

    /// Inspect the staged file before publication (`inspectStaged`).
    pub fn inspect_staged(&mut self, inspect: impl FnOnce(&Path)) -> Result<(), DownloadError> {
        if self.ended {
            return Err(DownloadError::SinkClosed);
        }
        if self.inspecting {
            return Err(DownloadError::Inspecting);
        }
        if let Err(error) = self.require_complete_ranges() {
            self.close();
            return Err(error);
        }
        self.inspecting = true;
        inspect(&self.staged);
        self.inspecting = false;
        if self.ended {
            return Err(DownloadError::ClosedDuringInspection);
        }
        Ok(())
    }

    /// Close the sink, removing its staged file (`close`).
    pub fn close(&mut self) {
        if self.ended {
            return;
        }
        self.ended = true;
        self.file = None;
        let _ = std::fs::remove_file(&self.staged);
    }
}

/// Download block (`DownloadBlock`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadBlock {
    /// Block index.
    pub index: u64,
    /// Total bytes on block zero.
    pub total_bytes: Option<u64>,
    /// Block bytes.
    pub bytes: Vec<u8>,
}

/// Q3 sliding download window (`DownloadWindow`).
pub struct DownloadWindow<'a> {
    source: &'a mut dyn DownloadSource,
    block_bytes: usize,
    window_blocks: u64,
    blocks: BTreeMap<u64, Vec<u8>>,
    read_offset: u64,
    current_block: u64,
    client_block: u64,
    transmit_block: u64,
    eof: bool,
    ended: bool,
    send_time: f64,
}

impl<'a> DownloadWindow<'a> {
    /// Create a window over a source.
    pub fn new(
        source: &'a mut dyn DownloadSource,
        block_bytes: usize,
        window_blocks: u64,
    ) -> Result<Self, DownloadError> {
        if block_bytes < 1 || window_blocks < 1 {
            return Err(DownloadError::BadWindow);
        }
        Ok(Self {
            source,
            block_bytes,
            window_blocks,
            blocks: BTreeMap::new(),
            read_offset: 0,
            current_block: 0,
            client_block: 0,
            transmit_block: 0,
            eof: false,
            ended: false,
            send_time: 0.0,
        })
    }

    /// True once closed.
    #[must_use]
    pub fn closed(&self) -> bool {
        self.ended
    }

    /// Next packets to transmit (`packets`).
    pub fn packets(&mut self, now: f64, mut count: u64) -> Result<Vec<DownloadBlock>, DownloadError> {
        if self.ended {
            return Err(DownloadError::WindowClosed);
        }
        while self.current_block - self.client_block < self.window_blocks
            && self.read_offset < self.source.byte_length()
        {
            let bytes = self.source.read(self.read_offset, self.block_bytes)?;
            if bytes.is_empty() {
                return Err(DownloadError::UnexpectedEof);
            }
            self.read_offset += bytes.len() as u64;
            self.blocks.insert(self.current_block, bytes);
            self.current_block += 1;
        }
        if self.read_offset == self.source.byte_length()
            && !self.eof
            && self.current_block - self.client_block < self.window_blocks
        {
            self.blocks.insert(self.current_block, Vec::new());
            self.current_block += 1;
            self.eof = true;
        }
        let mut packets = Vec::new();
        while count > 0 && self.client_block != self.current_block {
            count -= 1;
            if self.transmit_block == self.current_block {
                if now - self.send_time <= 1000.0 {
                    break;
                }
                self.transmit_block = self.client_block;
            }
            let bytes = self
                .blocks
                .get(&self.transmit_block)
                .ok_or(DownloadError::BlockNotRetained)?;
            packets.push(DownloadBlock {
                index: self.transmit_block,
                total_bytes: if self.transmit_block == 0 {
                    Some(self.source.byte_length())
                } else {
                    None
                },
                bytes: bytes.clone(),
            });
            self.transmit_block += 1;
            self.send_time = now;
        }
        Ok(packets)
    }

    /// Acknowledge a block (`acknowledge`).
    pub fn acknowledge(&mut self, block: u64, now: f64) -> Result<bool, DownloadError> {
        if self.ended || block != self.client_block || block >= self.transmit_block {
            return Err(DownloadError::BadAcknowledgement);
        }
        let bytes = self.blocks.remove(&block).ok_or(DownloadError::MissingBlock)?;
        if bytes.is_empty() {
            self.close();
            return Ok(true);
        }
        self.client_block += 1;
        self.send_time = now;
        Ok(false)
    }

    /// Close the window and its source (`close`).
    pub fn close(&mut self) {
        if !self.ended {
            self.ended = true;
            self.blocks.clear();
            self.source.close();
        }
    }
}

/// Stream an HTTP(S) response body into a sink (`downloadHttp`).
///
/// The host performs the request and hands over the validated response body;
/// only `http:` and `https:` URLs are accepted.
pub fn download_body(
    url: &str,
    sink: &mut DownloadSink,
    body: &mut dyn Read,
) -> Result<ContentDigest, DownloadError> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        sink.close();
        return Err(DownloadError::BadScheme);
    }
    let mut buffer = [0u8; 65536];
    loop {
        let count = body
            .read(&mut buffer)
            .map_err(|error| DownloadError::Filesystem(error.to_string()))?;
        if count == 0 {
            break;
        }
        if let Err(error) = sink.append(&buffer[..count]) {
            sink.close();
            return Err(error);
        }
    }
    sink.finish()
}

/// Missing pure-content digests (`verifyPureContent`).
#[must_use]
pub fn verify_pure_content(
    required: &[ContentDigest],
    available: &std::collections::HashSet<ContentDigest>,
) -> Vec<ContentDigest> {
    required.iter().filter(|digest| !available.contains(digest)).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_stay_contained() {
        assert!(download_path("a/b.pk3").is_ok());
        assert!(download_path("../a").is_err());
        assert!(download_path("a//b").is_err());
        assert!(download_path("a\\b").is_err());
        assert!(download_path("").is_err());
    }

    #[test]
    fn window_transmits_until_eof() {
        let mut source = MemorySource::new(vec![1, 2, 3, 4, 5]);
        let mut window = DownloadWindow::new(&mut source, 2, 8).unwrap();
        let mut now = 0.0;
        let mut received = Vec::new();
        loop {
            let packets = window.packets(now, 8).unwrap();
            assert!(!packets.is_empty());
            let mut complete = false;
            for packet in packets {
                if packet.bytes.is_empty() {
                    complete = window.acknowledge(packet.index, now).unwrap();
                } else {
                    received.extend_from_slice(&packet.bytes);
                    assert!(!window.acknowledge(packet.index, now).unwrap());
                }
                now += 10.0;
            }
            if complete {
                break;
            }
        }
        assert_eq!(received, vec![1, 2, 3, 4, 5]);
        assert!(window.closed());
    }

    #[test]
    fn sink_verifies_and_publishes() {
        let root = std::env::temp_dir().join(format!("qa-net-sink-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let digest = ContentDigest::new(
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        )
        .unwrap();
        let mut sink = DownloadSink::create(
            &root,
            "a/b.bin",
            SinkExpectation::Content(DownloadExpectation {
                digest: digest.clone(),
                byte_length: 3,
            }),
        )
        .unwrap();
        sink.append(b"abc").unwrap();
        assert_eq!(sink.finish().unwrap(), digest);
        assert_eq!(std::fs::read(root.join("a/b.bin")).unwrap(), b"abc");
        let _ = std::fs::remove_dir_all(&root);
    }
}
