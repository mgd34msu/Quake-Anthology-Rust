//! RoQ stream framing: chunk dispatch over the retained file buffer.
//!
//! Donor provenance: `src/media/roq-stream.ts` (`RoqStream`,
//! `CIN_PlayCinematic`, `RoQInterrupt` and `RoQReset` from id Software
//! `code/client/cl_cin.c`, Copyright (C) 1999-2005 Id Software, Inc.
//! GPL-2.0-or-later).
//!
//! The donor threads one `cin.file` allocation through both the stream
//! and the decoder scratch. Only the stream writes that allocation
//! (outside explicit clears), so this port gives the stream its own
//! buffer with identical contents; [`RoqStream::clear_buffer`]
//! reproduces the shared-allocation clear on playback reset.

use super::containers::RoqEndPolicy;
use super::source::MediaInput;
use crate::ClientError;

/// Retained stream buffer length (`65536 + 8`).
pub const ROQ_STREAM_BUFFER: usize = 65536 + 8;
/// Largest chunk payload the lookahead accepts.
pub const ROQ_MAX_CHUNK: usize = 65536;

/// Reopen hook for file-backed streams (returns `None` when the
/// source went missing).
pub type RoqOpener = Box<dyn FnMut() -> Option<Box<dyn MediaInput>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamKind {
    Open,
    Missing,
    Bytes,
    Closed,
}

enum StreamFile {
    Open {
        opener: RoqOpener,
        input: Box<dyn MediaInput>,
    },
    Missing {
        opener: RoqOpener,
    },
    Bytes {
        data: Vec<u8>,
    },
    Closed,
}

impl StreamFile {
    const fn kind(&self) -> StreamKind {
        match self {
            Self::Open { .. } => StreamKind::Open,
            Self::Missing { .. } => StreamKind::Missing,
            Self::Bytes { .. } => StreamKind::Bytes,
            Self::Closed => StreamKind::Closed,
        }
    }
}

/// A RoQ chunk header (`ChunkHeader`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoqChunkHeader {
    /// Chunk id.
    pub id: u16,
    /// Payload size (three size bytes).
    pub size: usize,
    /// Flags.
    pub flags: u16,
}

/// A dispatched RoQ chunk (`RoqStreamChunk`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoqStreamChunk {
    /// Chunk id.
    pub id: u16,
    /// Payload size.
    pub size: usize,
    /// Flags.
    pub flags: u16,
    /// Readable bytes: the payload, extended into retained buffer
    /// bytes for codebook chunks under the cinematic lookahead.
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingChunk {
    header: RoqChunkHeader,
    offset: usize,
    buffered: bool,
}

/// One unique source file, with only the current payload and
/// following header retained (`RoqStream`).
pub struct RoqStream {
    source: String,
    length: usize,
    file: StreamFile,
    buffer: Vec<u8>,
    end_policy: RoqEndPolicy,
    position: usize,
    played: usize,
    header: Option<[u8; 8]>,
    next_header: Option<RoqChunkHeader>,
    invalid_lookahead: bool,
    retained_eof: bool,
    reset_header: bool,
    in_memory: u16,
    buffered_next: bool,
    chunk_offset: usize,
    buffer_offset: usize,
    buffered_length: usize,
    pending: Option<PendingChunk>,
}

impl RoqStream {
    fn new(source: &str, length: usize, file: StreamFile, end_policy: RoqEndPolicy) -> Self {
        Self {
            source: source.to_string(),
            length,
            file,
            buffer: vec![0u8; ROQ_STREAM_BUFFER],
            end_policy,
            position: 0,
            played: 24,
            header: None,
            next_header: None,
            invalid_lookahead: false,
            retained_eof: false,
            reset_header: false,
            in_memory: 0,
            buffered_next: false,
            chunk_offset: 0,
            buffer_offset: 0,
            buffered_length: 0,
            pending: None,
        }
    }

    /// Open a file-backed stream (`RoqStream.open`); returns `None`
    /// for missing or empty sources.
    pub fn open(mut opener: RoqOpener, source: &str) -> Option<Self> {
        let input = opener()?;
        if input.byte_length() == 0 {
            return None;
        }
        let length = input.byte_length();
        Some(Self::new(
            source,
            length,
            StreamFile::Open { opener, input },
            RoqEndPolicy::CinematicLookahead,
        ))
    }

    /// Open a diagnostic byte stream (`RoqStream.fromBytes`).
    pub fn from_bytes(data: Vec<u8>, source: &str, end_policy: RoqEndPolicy) -> Self {
        let length = data.len();
        Self::new(source, length, StreamFile::Bytes { data }, end_policy)
    }

    /// Source name.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Capture a checkpoint.
    pub fn capture_checkpoint(&self) -> Result<RoqStreamCheckpoint, ClientError> {
        if self.pending.is_some() {
            return Err(ClientError::BadMedia(
                "Cannot checkpoint a pending RoQ chunk dispatch".to_string(),
            ));
        }
        Ok(RoqStreamCheckpoint {
            length: self.length,
            end_policy: self.end_policy,
            closed: self.file.kind() == StreamKind::Closed,
            missing: self.file.kind() == StreamKind::Missing,
            position: self.position,
            played: self.played,
            header: self.header,
            next_header: self.next_header,
            invalid_lookahead: self.invalid_lookahead,
            retained_eof: self.retained_eof,
            reset_header: self.reset_header,
            in_memory: self.in_memory,
            buffered_next: self.buffered_next,
            chunk_offset: self.chunk_offset,
            buffer_offset: self.buffer_offset,
            buffered_length: self.buffered_length,
            buffer: self.buffer.clone(),
        })
    }

    /// Restore a checkpoint.
    pub fn restore_checkpoint(&mut self, checkpoint: &RoqStreamCheckpoint) -> Result<(), ClientError> {
        if self.pending.is_some() {
            return Err(ClientError::BadMedia(
                "Cannot restore a pending RoQ chunk dispatch".to_string(),
            ));
        }
        if checkpoint.length != self.length {
            return Err(ClientError::BadMedia("RoQ stream length differs".to_string()));
        }
        if checkpoint.end_policy != self.end_policy {
            return Err(ClientError::BadMedia("RoQ end policy differs".to_string()));
        }
        if checkpoint.buffer.len() != self.buffer.len() {
            return Err(ClientError::BadMedia("scratch buffer size differs".to_string()));
        }
        if checkpoint.position > self.length || checkpoint.buffered_length > self.buffer.len() {
            return Err(ClientError::BadMedia("stream cursor outside input".to_string()));
        }
        if checkpoint.missing && self.file.kind() != StreamKind::Missing {
            return Err(ClientError::BadMedia("missing source is no longer missing".to_string()));
        }
        self.played = checkpoint.played;
        self.in_memory = checkpoint.in_memory;
        self.chunk_offset = checkpoint.chunk_offset;
        self.buffer_offset = checkpoint.buffer_offset;
        self.invalid_lookahead = checkpoint.invalid_lookahead;
        self.retained_eof = checkpoint.retained_eof;
        self.reset_header = checkpoint.reset_header;
        self.buffered_next = checkpoint.buffered_next;
        self.position = checkpoint.position;
        self.buffered_length = checkpoint.buffered_length;
        self.header = checkpoint.header;
        self.next_header = checkpoint.next_header;
        self.buffer.copy_from_slice(&checkpoint.buffer);
        if checkpoint.closed {
            self.close();
        }
        Ok(())
    }

    /// Read the file header (`initialize`).
    pub fn initialize(&mut self) -> Result<[u8; 8], ClientError> {
        if let Some(header) = self.header {
            return Ok(header);
        }
        let count = self.read(16)?;
        let bytes_short = matches!(self.file, StreamFile::Bytes { .. }) && count == 8 && self.length == 8;
        if count != 16 && !bytes_short {
            return Err(ClientError::BadMedia(format!(
                "{}:{}: truncated RoQ header",
                self.source, self.position
            )));
        }
        self.buffer_offset = self.position - count;
        self.buffered_length = count;
        let mut header = [0u8; 8];
        header.copy_from_slice(&self.buffer[..8]);
        self.next_header = if count == 8 { None } else { Some(self.parse_header(8)?) };
        if self.next_header.is_some_and(|next| next.size > ROQ_MAX_CHUNK) {
            return Err(ClientError::BadMedia(format!(
                "{}:10: RoQ chunk exceeds 65536 bytes",
                self.source
            )));
        }
        self.header = Some(header);
        Ok(header)
    }

    /// Resume after a menu callback (`beginPlayback`).
    pub fn begin_playback(&mut self) -> Result<(), ClientError> {
        self.header = None;
        self.reset_header = false;
        self.played = 24;
        self.invalid_lookahead = false;
        self.retained_eof = false;
        self.buffered_next = false;
        self.pending = None;
        self.initialize().map(|_| ())
    }

    /// Whether the header came from a reset (`initializedByReset`).
    #[must_use]
    pub const fn initialized_by_reset(&self) -> bool {
        self.reset_header
    }

    /// Whether the lookahead is invalid (`hasInvalidLookahead`).
    #[must_use]
    pub const fn has_invalid_lookahead(&self) -> bool {
        self.invalid_lookahead
    }

    /// Whether the next chunk is buffered in a packet (`inPacket`).
    #[must_use]
    pub const fn in_packet(&self) -> bool {
        self.buffered_next
    }

    /// Dispatch the next chunk (`nextChunk`).
    pub fn next_chunk(&mut self) -> Result<Option<RoqStreamChunk>, ClientError> {
        if self.pending.is_some() {
            return Err(ClientError::BadMedia(
                "RoQ chunk dispatch has not completed".to_string(),
            ));
        }
        if self.invalid_lookahead {
            return Ok(None);
        }
        if self.header.is_none() {
            return Err(ClientError::BadMedia("RoQ stream has not been initialized".to_string()));
        }
        let Some(header) = self.next_header else {
            return Ok(None);
        };
        if !self.buffered_next {
            let offset = self.position;
            let count = self.read(header.size + 8)?;
            // `RoQInterrupt` performs this read even when `RoQPlayed`
            // already names EOF.
            if self.end_policy == RoqEndPolicy::CinematicLookahead && self.played >= self.length {
                return Ok(None);
            }
            if self.end_policy == RoqEndPolicy::Complete {
                if count < header.size {
                    return Err(ClientError::BadMedia(format!(
                        "{}:{}: truncated RoQ payload",
                        self.source,
                        offset + count
                    )));
                }
                self.buffered_length = count;
            } else {
                if count < header.size + 8 {
                    // Leading audio can leave a complete final payload
                    // and retained lookahead in the file buffer.
                    if count == header.size && self.position == self.length {
                        self.retained_eof = true;
                    } else if count != 0 || !self.retained_eof {
                        return Err(ClientError::BadMedia(format!(
                            "{}:{}: truncated RoQ payload or lookahead header",
                            self.source,
                            offset + count
                        )));
                    }
                }
                if count == header.size + 8 && self.position == self.length {
                    self.retained_eof = true;
                }
                self.buffered_length = header.size + 8;
            }
            self.buffer_offset = offset;
            self.chunk_offset = 0;
        }
        let offset = self.chunk_offset;
        let decoded_size = if header.id == 0x1030 || header.id == 0x1013 {
            0
        } else {
            header.size
        };
        let limit = if self.buffered_next {
            self.buffer.len().min(ROQ_MAX_CHUNK)
        } else {
            self.buffered_length
        };
        if offset + decoded_size > limit {
            return Err(ClientError::BadMedia(format!(
                "{}:{}: RoQ packet payload exceeds source scratch buffer",
                self.source,
                self.buffer_offset + offset
            )));
        }
        self.pending = Some(PendingChunk {
            header,
            offset,
            buffered: self.buffered_next,
        });
        // `decodeCodeBook` follows its entry counts beyond the chunk
        // into retained file bytes.
        let end = if header.id == 0x1002 && self.end_policy == RoqEndPolicy::CinematicLookahead {
            self.buffer.len().min(ROQ_MAX_CHUNK)
        } else {
            offset + decoded_size
        };
        Ok(Some(RoqStreamChunk {
            id: header.id,
            size: header.size,
            flags: header.flags,
            bytes: self.buffer[offset..end].to_vec(),
        }))
    }

    /// Finish the reached dispatch (`completeChunk`).
    pub fn complete_chunk(&mut self, ended: bool) -> Result<(), ClientError> {
        let Some(pending) = self.pending.take() else {
            return Err(ClientError::BadMedia("RoQ has no pending chunk dispatch".to_string()));
        };
        let header = pending.header;
        let offset = pending.offset;
        if header.id == 0x1030 {
            self.in_memory = header.flags;
        }
        let following = offset
            + if header.id == 0x1030 || header.id == 0x1013 {
                0
            } else {
                header.size
            };
        if self.end_policy == RoqEndPolicy::Complete && following == self.buffered_length && self.in_memory == 0 {
            self.next_header = None;
            self.buffered_next = false;
            return Ok(());
        }
        // Redump may reach retained bytes after a reset.
        let limit = if pending.buffered {
            self.buffer.len().min(ROQ_MAX_CHUNK)
        } else {
            self.buffered_length
        };
        if following + 8 > limit {
            return Err(ClientError::BadMedia(format!(
                "{}:{}: truncated RoQ packet or lookahead header",
                self.source,
                self.buffer_offset + following
            )));
        }
        let next = self.parse_header(following)?;
        self.next_header = Some(next);
        self.invalid_lookahead = next.size > ROQ_MAX_CHUNK || next.id == 0x1084;
        self.buffered_next = false;
        if self.invalid_lookahead {
            return Ok(());
        }
        if self.in_memory != 0 && !ended {
            self.in_memory -= 1;
            self.chunk_offset = following + 8;
            self.buffered_next = true;
        } else {
            self.played += next.size + 8;
        }
        Ok(())
    }

    /// Reset the stream (`rewind`/`RoQReset`).
    pub fn rewind(&mut self) -> Result<(), ClientError> {
        let kind = self.file.kind();
        if kind == StreamKind::Closed {
            return Err(ClientError::BadMedia("Cannot reset a closed RoQ file".to_string()));
        }
        if kind != StreamKind::Bytes {
            let old = std::mem::replace(&mut self.file, StreamFile::Closed);
            let mut opener = match old {
                StreamFile::Open { opener, .. } | StreamFile::Missing { opener } => opener,
                _ => unreachable!(),
            };
            self.file = match opener() {
                None => StreamFile::Missing { opener },
                Some(input) => StreamFile::Open { opener, input },
            };
        }
        self.position = 0;
        self.played = 24;
        self.invalid_lookahead = false;
        self.retained_eof = false;
        self.buffered_next = false;
        self.pending = None;
        // `RoQReset` ignores the read count and does not recheck the
        // magic, size word or first chunk size.
        let count = self.read(16)?;
        self.buffer_offset = 0;
        self.buffered_length = 16;
        let mut header = [0u8; 8];
        header.copy_from_slice(&self.buffer[..8]);
        self.header = Some(header);
        self.next_header = if matches!(self.file, StreamFile::Bytes { .. }) && count == 8 && self.length == 8 {
            None
        } else {
            Some(self.parse_header(8)?)
        };
        self.reset_header = true;
        Ok(())
    }

    /// Close the stream.
    pub fn close(&mut self) {
        self.file = StreamFile::Closed;
    }

    /// Retire the stream without running table closure (`retire`).
    pub fn retire(&mut self) {
        self.file = StreamFile::Closed;
    }

    /// Zero the retained buffer (the donor clears the shared
    /// `cin.file` allocation through the decoder scratch on reset).
    pub fn clear_buffer(&mut self) {
        self.buffer.fill(0);
    }

    fn read(&mut self, length: usize) -> Result<usize, ClientError> {
        if self.file.kind() == StreamKind::Closed {
            return Err(ClientError::BadMedia("RoQ file is closed".to_string()));
        }
        if length > self.buffer.len() {
            return Err(ClientError::BadMedia(format!(
                "{}:{}: RoQ chunk exceeds source scratch buffer",
                self.source, self.position
            )));
        }
        let count = match &mut self.file {
            StreamFile::Bytes { data } => {
                let end = (self.position + length).min(data.len());
                let start = self.position.min(data.len());
                let count = end - start;
                self.buffer[..count].copy_from_slice(&data[start..end]);
                count
            }
            StreamFile::Missing { .. } => 0,
            StreamFile::Open { input, .. } => input.read_at(self.position, &mut self.buffer[..length])?,
            StreamFile::Closed => unreachable!(),
        };
        self.position += count;
        Ok(count)
    }

    fn parse_header(&self, offset: usize) -> Result<RoqChunkHeader, ClientError> {
        let Some(window) = self.buffer.get(offset..offset + 8) else {
            return Err(ClientError::BadMedia(format!(
                "{}:{}: truncated RoQ packet or lookahead header",
                self.source,
                self.buffer_offset + offset
            )));
        };
        let id = u16::from_le_bytes([window[0], window[1]]);
        // The source reads three size bytes and skips the fourth.
        let size = u32::from_le_bytes([window[2], window[3], window[4], window[5]]) & 0x00ff_ffff;
        let flags = u16::from_le_bytes([window[6], window[7]]);
        Ok(RoqChunkHeader {
            id,
            size: size as usize,
            flags,
        })
    }
}

/// A RoQ stream checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoqStreamCheckpoint {
    /// Input length.
    pub length: usize,
    /// End policy.
    pub end_policy: RoqEndPolicy,
    /// Closed.
    pub closed: bool,
    /// Missing source.
    pub missing: bool,
    /// Read position.
    pub position: usize,
    /// Played count.
    pub played: usize,
    /// Retained file header.
    pub header: Option<[u8; 8]>,
    /// Lookahead header.
    pub next_header: Option<RoqChunkHeader>,
    /// Invalid lookahead.
    pub invalid_lookahead: bool,
    /// Retained EOF.
    pub retained_eof: bool,
    /// Header came from a reset.
    pub reset_header: bool,
    /// Remaining in-memory chunks.
    pub in_memory: u16,
    /// Next chunk is buffered.
    pub buffered_next: bool,
    /// Chunk offset.
    pub chunk_offset: usize,
    /// Buffer file offset.
    pub buffer_offset: usize,
    /// Buffered length.
    pub buffered_length: usize,
    /// Retained buffer.
    pub buffer: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::source::MemMedia;

    fn chunk(id: u16, size: usize, flags: u16, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8; 8 + payload.len()];
        out[..2].copy_from_slice(&id.to_le_bytes());
        out[2..6].copy_from_slice(&(size as u32).to_le_bytes());
        out[6..8].copy_from_slice(&flags.to_le_bytes());
        out[8..].copy_from_slice(payload);
        out
    }

    fn header() -> Vec<u8> {
        vec![0x84, 0x10, 0, 0, 0, 0, 30, 0]
    }

    fn cat(parts: &[Vec<u8>]) -> Vec<u8> {
        parts.concat()
    }

    fn drain(stream: &mut RoqStream) -> Vec<(u16, usize, u16, usize, bool)> {
        let mut chunks = Vec::new();
        while let Some(chunk) = stream.next_chunk().unwrap() {
            chunks.push((chunk.id, chunk.size, chunk.flags, chunk.bytes.len(), stream.in_packet()));
            stream.complete_chunk(false).unwrap();
        }
        chunks
    }

    #[test]
    fn walks_chunks_under_both_policies() {
        let bytes = cat(&[
            header(),
            chunk(0x1001, 8, 0, &[8, 0, 8, 0, 0, 0, 0, 0]),
            chunk(0x1002, 6, 0x0100, &[1, 2, 3, 4, 5, 6]),
            chunk(0x1011, 3, 0, &[0, 0x80, 0]),
            chunk(0x1020, 2, 7, &[9, 9]),
        ]);
        for policy in [RoqEndPolicy::Complete, RoqEndPolicy::CinematicLookahead] {
            let mut stream = RoqStream::from_bytes(bytes.clone(), "<test>", policy);
            assert_eq!(stream.initialize().unwrap(), [0x84, 0x10, 0, 0, 0, 0, 30, 0]);
            let chunks = drain(&mut stream);
            // The lookahead names EOF once `played` reaches the input
            // length, so the trailing audio chunk never dispatches
            // (donor behavior, verified under bun).
            let expected = if policy == RoqEndPolicy::Complete {
                vec![
                    (0x1001, 8, 0, 8, false),
                    (0x1002, 6, 0x0100, 6, false),
                    (0x1011, 3, 0, 3, false),
                    (0x1020, 2, 7, 2, false),
                ]
            } else {
                vec![
                    (0x1001, 8, 0, 8, false),
                    // Codebook chunks read the retained buffer.
                    (0x1002, 6, 0x0100, ROQ_MAX_CHUNK, false),
                    (0x1011, 3, 0, 3, false),
                ]
            };
            assert_eq!(chunks, expected, "policy {policy:?}");
            assert!(!stream.has_invalid_lookahead());
        }
    }

    #[test]
    fn codebook_reads_retained_bytes_under_lookahead() {
        let bytes = cat(&[
            header(),
            chunk(0x1001, 8, 0, &[8, 0, 8, 0, 0, 0, 0, 0]),
            chunk(0x1002, 6, 0x0100, &[1, 2, 3, 4, 5, 6]),
            chunk(0x1011, 2, 0, &[7, 7]),
        ]);
        let mut stream = RoqStream::from_bytes(bytes, "<test>", RoqEndPolicy::CinematicLookahead);
        stream.initialize().unwrap();
        stream.next_chunk().unwrap();
        stream.complete_chunk(false).unwrap();
        let codebook = stream.next_chunk().unwrap().unwrap();
        assert_eq!(codebook.id, 0x1002);
        assert_eq!(codebook.bytes.len(), ROQ_MAX_CHUNK);
        assert_eq!(&codebook.bytes[..6], &[1, 2, 3, 4, 5, 6]);
        // The following chunk header is retained in the same buffer.
        assert_eq!(codebook.bytes[6], 0x11);
        stream.complete_chunk(false).unwrap();
    }

    #[test]
    fn packet_trace_matches_donor() {
        // Oracle sequence from the donor under bun.
        let bytes = cat(&[
            header(),
            chunk(0x1001, 8, 0, &[8, 0, 8, 0, 0, 0, 0, 0]),
            chunk(0x1030, 16, 1, &[]),
            chunk(0x1011, 8, 0, &[1, 2, 3, 4, 5, 6, 7, 8]),
            chunk(0x1011, 8, 0, &[9, 9, 9, 9, 9, 9, 9, 9]),
        ]);
        let mut stream = RoqStream::from_bytes(bytes, "<test>", RoqEndPolicy::CinematicLookahead);
        stream.initialize().unwrap();
        let chunks = drain(&mut stream);
        assert_eq!(
            chunks,
            vec![
                (0x1001, 8, 0, 8, false),
                (0x1030, 16, 1, 0, false),
                (0x1011, 8, 0, 8, true)
            ]
        );
        // `played` names EOF before the trailing frame (donor ends
        // clean here too).
        assert!(!stream.has_invalid_lookahead());
    }

    #[test]
    fn oversized_lookahead_invalidates() {
        let bytes = cat(&[
            header(),
            chunk(0x1001, 8, 0, &[8, 0, 8, 0, 0, 0, 0, 0]),
            chunk(0x1011, 70000, 0, &[]),
        ]);
        for policy in [RoqEndPolicy::Complete, RoqEndPolicy::CinematicLookahead] {
            let mut stream = RoqStream::from_bytes(bytes.clone(), "<test>", policy);
            stream.initialize().unwrap();
            stream.next_chunk().unwrap();
            stream.complete_chunk(false).unwrap();
            assert!(stream.has_invalid_lookahead(), "policy {policy:?}");
            assert!(stream.next_chunk().unwrap().is_none());
        }
    }

    #[test]
    fn complete_policy_ends_cleanly_on_short_reads() {
        let bytes = cat(&[header(), chunk(0x1001, 8, 0, &[8, 0, 8, 0, 0, 0, 0, 0])]);
        let mut stream = RoqStream::from_bytes(bytes, "<test>", RoqEndPolicy::Complete);
        stream.initialize().unwrap();
        let chunks = drain(&mut stream);
        assert_eq!(chunks.len(), 1);
        assert!(!stream.has_invalid_lookahead());
    }

    #[test]
    fn rewind_and_checkpoint_round_trip() {
        let bytes = cat(&[
            header(),
            chunk(0x1001, 8, 0, &[8, 0, 8, 0, 0, 0, 0, 0]),
            chunk(0x1011, 2, 0, &[1, 2]),
        ]);
        let mut stream = RoqStream::from_bytes(bytes.clone(), "<test>", RoqEndPolicy::Complete);
        stream.initialize().unwrap();
        stream.next_chunk().unwrap();
        stream.complete_chunk(false).unwrap();
        let checkpoint = stream.capture_checkpoint().unwrap();
        let mut revived = RoqStream::from_bytes(bytes, "<test>", RoqEndPolicy::Complete);
        revived.restore_checkpoint(&checkpoint).unwrap();
        let chunk = revived.next_chunk().unwrap().unwrap();
        assert_eq!((chunk.id, chunk.size), (0x1011, 2));
        revived.complete_chunk(false).unwrap();
        assert!(revived.next_chunk().unwrap().is_none());
        revived.rewind().unwrap();
        assert!(revived.initialized_by_reset());
        let chunk = revived.next_chunk().unwrap().unwrap();
        assert_eq!(chunk.id, 0x1001);
    }

    #[test]
    fn openers_cover_files_and_missing_sources() {
        // File-backed streams use the cinematic lookahead, so the
        // fixture extends past the first chunk (tiny inputs name EOF
        // immediately).
        let bytes = cat(&[
            header(),
            chunk(0x1001, 8, 0, &[8, 0, 8, 0, 0, 0, 0, 0]),
            chunk(0x1011, 2, 0, &[1, 2]),
        ]);
        let owned = bytes.clone();
        let mut stream = RoqStream::open(
            Box::new(move || Some(Box::new(MemMedia::new(owned.clone(), "<mem>")) as Box<dyn MediaInput>)),
            "<test>",
        )
        .unwrap();
        stream.initialize().unwrap();
        assert_eq!(drain(&mut stream).len(), 1);
        let none: RoqOpener = Box::new(|| None);
        assert!(RoqStream::open(none, "<test>").is_none());
        assert!(RoqStream::open(
            Box::new(|| Some(Box::new(MemMedia::new(Vec::new(), "<mem>")) as Box<dyn MediaInput>)),
            "<test>",
        )
        .is_none());
        // A source that goes missing rewinds into the missing state,
        // where reads return zero bytes and dispatch fails.
        let owned = bytes.clone();
        let mut calls = 0;
        let mut stream = RoqStream::open(
            Box::new(move || {
                calls += 1;
                if calls > 1 {
                    return None;
                }
                Some(Box::new(MemMedia::new(owned.clone(), "<mem>")) as Box<dyn MediaInput>)
            }),
            "<test>",
        )
        .unwrap();
        stream.initialize().unwrap();
        stream.rewind().unwrap();
        assert!(stream.next_chunk().is_err());
    }

    #[test]
    fn begin_and_retire() {
        let bytes = cat(&[header(), chunk(0x1001, 8, 0, &[8, 0, 8, 0, 0, 0, 0, 0])]);
        let mut stream = RoqStream::from_bytes(bytes, "<test>", RoqEndPolicy::Complete);
        stream.begin_playback().unwrap();
        assert!(stream.next_chunk().unwrap().is_some());
        stream.retire();
        assert!(stream.next_chunk().is_err());
    }

    #[test]
    fn dispatch_protocol_is_checked() {
        let bytes = cat(&[header(), chunk(0x1001, 8, 0, &[8, 0, 8, 0, 0, 0, 0, 0])]);
        let mut stream = RoqStream::from_bytes(bytes, "<test>", RoqEndPolicy::Complete);
        assert!(stream.next_chunk().is_err());
        stream.initialize().unwrap();
        assert!(stream.complete_chunk(false).is_err());
        stream.next_chunk().unwrap();
        assert!(stream.next_chunk().is_err());
        assert!(stream.capture_checkpoint().is_err());
        stream.complete_chunk(false).unwrap();
        stream.close();
        assert!(stream.rewind().is_err());
    }
}
