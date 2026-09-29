//! RoQ decoder: VQ frames and RLL audio over a chunk stream.
//!
//! Donor provenance: `src/media/roq.ts` (`RoqDecoder`,
//! `RoqDecoderScratch`, ported from id Software
//! `code/client/cl_cin.c`, Copyright (C) 1999-2005 Id Software, Inc.
//! GPL-2.0-or-later).
//!
//! The donor's front/back images are live views into the shared
//! physical buffer; this port addresses the same buffer by offset so
//! the borrow checker can see the disjoint motion-source and
//! destination ranges.

use qa_core::binary::{BinaryError, BinaryReader};

use super::containers::RoqEndPolicy;
use super::roq_audio::SourceRoqAudio;
use super::roq_codebook::{RoqCodebookFormat, RoqCodebookMode, RoqCodebookProfile, SourceRoqCodebooks};
use super::roq_stream::{RoqStream, RoqStreamCheckpoint};
use crate::ClientError;

/// Physical frame buffer length (`512 * 512 * 4 * 2`).
pub const ROQ_PHYSICAL_FRAMES: usize = 512 * 512 * 4 * 2;
/// Retained file buffer length (`65536 + 8`).
pub const ROQ_FILE_BUFFER: usize = 65536 + 8;
/// Largest frame image (`512 * 512 * 4`).
pub const ROQ_MAX_IMAGE: usize = 512 * 512 * 4;
/// RoQ audio sample rate.
pub const ROQ_SAMPLE_RATE: u32 = 22050;

fn map_err(error: BinaryError) -> ClientError {
    ClientError::BadMedia(error.to_string())
}

/// Process-wide codebooks and fixed frame buffers
/// (`RoqDecoderScratch`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoqDecoderScratch {
    file: Vec<u8>,
    codebooks: SourceRoqCodebooks,
    frames: Vec<u8>,
}

impl Default for RoqDecoderScratch {
    fn default() -> Self {
        Self::new()
    }
}

impl RoqDecoderScratch {
    /// Zeroed scratch.
    #[must_use]
    pub fn new() -> Self {
        Self {
            file: vec![0u8; ROQ_FILE_BUFFER],
            codebooks: SourceRoqCodebooks::new(),
            frames: vec![0u8; ROQ_PHYSICAL_FRAMES],
        }
    }

    /// Retained file bytes.
    #[must_use]
    pub fn file(&self) -> &[u8] {
        &self.file
    }

    /// Codebooks.
    #[must_use]
    pub const fn codebooks(&self) -> &SourceRoqCodebooks {
        &self.codebooks
    }

    /// Mutable codebooks.
    pub fn codebooks_mut(&mut self) -> &mut SourceRoqCodebooks {
        &mut self.codebooks
    }

    /// 2x2 book bytes.
    #[must_use]
    pub fn book2(&self) -> &[u8] {
        self.codebooks.book2()
    }

    /// 4x4 book bytes.
    #[must_use]
    pub fn book4(&self) -> &[u8] {
        self.codebooks.book4()
    }

    /// 8x8 book bytes.
    #[must_use]
    pub fn book8(&self) -> &[u8] {
        self.codebooks.book8()
    }

    /// Capture a checkpoint.
    #[must_use]
    pub fn capture_checkpoint(&self) -> RoqScratchCheckpoint {
        RoqScratchCheckpoint {
            file: self.file.clone(),
            book2: self.codebooks.book2().to_vec(),
            book4: self.codebooks.book4().to_vec(),
            book8: self.codebooks.book8().to_vec(),
            frames: self.frames.clone(),
        }
    }

    /// Restore a checkpoint.
    pub fn restore_checkpoint(&mut self, checkpoint: &RoqScratchCheckpoint) -> Result<(), ClientError> {
        if checkpoint.file.len() != self.file.len()
            || checkpoint.book2.len() != self.codebooks.book2().len()
            || checkpoint.book4.len() != self.codebooks.book4().len()
            || checkpoint.book8.len() != self.codebooks.book8().len()
            || checkpoint.frames.len() != self.frames.len()
        {
            return Err(ClientError::BadMedia("scratch allocation length differs".to_string()));
        }
        self.file.copy_from_slice(&checkpoint.file);
        self.codebooks.book2_mut().copy_from_slice(&checkpoint.book2);
        self.codebooks.book4_mut().copy_from_slice(&checkpoint.book4);
        self.codebooks.book8_mut().copy_from_slice(&checkpoint.book8);
        self.frames.copy_from_slice(&checkpoint.frames);
        Ok(())
    }

    /// Clear per-movie state (the global codebooks survive).
    pub fn clear_movie_state(&mut self) {
        self.file.fill(0);
        self.frames.fill(0);
    }

    /// Clear movie state and codebooks.
    pub fn clear(&mut self) {
        self.clear_movie_state();
        self.codebooks.clear();
    }

    /// Shared physical image view: the second image starts at the
    /// movie's frame length, not a fixed half-buffer offset.
    pub fn frame(&mut self, byte_length: usize, index: u8) -> Result<&mut [u8], ClientError> {
        if byte_length == 0 || byte_length > ROQ_MAX_IMAGE || index > 1 {
            return Err(ClientError::BadMedia("Invalid RoQ scratch frame length".to_string()));
        }
        let offset = byte_length * usize::from(index);
        self.frames
            .get_mut(offset..offset + byte_length)
            .ok_or_else(|| ClientError::BadMedia("Invalid RoQ scratch frame length".to_string()))
    }

    /// Read a physical buffer range (presentation may read beyond the
    /// published frame into the same buffer).
    pub fn view(&self, offset: usize, byte_length: usize) -> Result<&[u8], ClientError> {
        self.frames
            .get(offset..offset + byte_length)
            .ok_or_else(|| ClientError::BadMedia("Invalid RoQ physical buffer range".to_string()))
    }
}

/// A RoQ decoder scratch checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoqScratchCheckpoint {
    /// File bytes.
    pub file: Vec<u8>,
    /// 2x2 book.
    pub book2: Vec<u8>,
    /// 4x4 book.
    pub book4: Vec<u8>,
    /// 8x8 book.
    pub book8: Vec<u8>,
    /// Physical frames.
    pub frames: Vec<u8>,
}

struct VqReader<'a> {
    reader: BinaryReader<'a>,
    codes: u16,
    remaining: u8,
}

impl<'a> VqReader<'a> {
    fn new(bytes: &'a [u8], source: &str) -> Self {
        Self {
            reader: BinaryReader::new(bytes, source),
            codes: 0,
            remaining: 0,
        }
    }

    fn code(&mut self) -> Result<u8, ClientError> {
        if self.remaining == 0 {
            self.codes = self.reader.u16().map_err(map_err)?;
            self.remaining = 8;
        }
        let code = (self.codes >> 14) as u8;
        self.codes = self.codes.wrapping_shl(2);
        self.remaining -= 1;
        Ok(code)
    }

    fn byte(&mut self) -> Result<u8, ClientError> {
        self.reader.u8().map_err(map_err)
    }
}

/// A decoded RoQ frame (`RoqEvent` frame).
#[derive(Debug, Clone, PartialEq)]
pub struct RoqFrameEvent {
    /// RGBA pixels.
    pub rgba: Vec<u8>,
    /// Frame index.
    pub index: i32,
    /// Source time in milliseconds.
    pub time: f64,
}

/// Decoded RoQ audio (`RoqEvent` audio).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoqAudioEvent {
    /// Samples.
    pub samples: Vec<i16>,
    /// Channels.
    pub channels: u8,
    /// Sample rate (`22050`).
    pub sample_rate: u32,
}

/// A decoder event (`RoqEvent`).
#[derive(Debug, Clone, PartialEq)]
pub enum RoqEvent {
    /// Video frame.
    Frame(RoqFrameEvent),
    /// Audio samples.
    Audio(RoqAudioEvent),
    /// End of stream.
    End,
}

/// A decoder chunk event (`RoqChunkEvent`).
#[derive(Debug, Clone, PartialEq)]
pub enum RoqChunkEvent {
    /// Video frame.
    Frame(RoqFrameEvent),
    /// Audio samples.
    Audio(RoqAudioEvent),
    /// End of stream.
    End,
    /// Quad info.
    Info {
        /// Width.
        width: usize,
        /// Height.
        height: usize,
    },
    /// Metadata chunk.
    Metadata,
}

/// Chunk hooks (`nextChunk` callbacks).
#[derive(Default)]
pub struct RoqChunkHooks<'a> {
    /// Runs before a stereo audio decode.
    pub before_stereo: Option<&'a mut dyn FnMut()>,
    /// Runs after an info decode.
    pub after_info: Option<&'a mut dyn FnMut()>,
    /// Runs after an audio decode.
    pub after_audio: Option<&'a mut dyn FnMut(&RoqAudioEvent)>,
}

/// Terminal unknown-chunk state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoqUnknownChunk {
    /// No unknown chunk seen.
    None,
    /// Interrupted by an invalid lookahead.
    Interrupt,
    /// Ended without an invalid lookahead.
    RunTail,
}

/// Decoder options (`RoqDecoderOptions`; the end policy applies to
/// byte inputs only, since streams carry their own policy).
#[derive(Debug)]
pub struct RoqDecoderOptions {
    /// End policy for byte inputs.
    pub end_policy: RoqEndPolicy,
    /// Silent (audio chunks decode as metadata).
    pub silent: bool,
    /// Shared scratch.
    pub scratch: Option<RoqDecoderScratch>,
}

impl Default for RoqDecoderOptions {
    fn default() -> Self {
        Self {
            end_policy: RoqEndPolicy::Complete,
            silent: false,
            scratch: None,
        }
    }
}

/// The RoQ decoder (`RoqDecoder`).
pub struct RoqDecoder {
    audio: SourceRoqAudio,
    rate: u32,
    stream: RoqStream,
    scratch: RoqDecoderScratch,
    frame_width: usize,
    frame_height: usize,
    frame_index: i32,
    unknown_chunk: RoqUnknownChunk,
    cinematic_lookahead: bool,
    silent: bool,
}

impl RoqDecoder {
    fn bind(
        source: &str,
        stream: RoqStream,
        options: RoqDecoderOptions,
        from_stream: bool,
    ) -> Result<Self, ClientError> {
        let mut decoder = Self {
            audio: SourceRoqAudio::new(),
            rate: 30,
            stream,
            scratch: options.scratch.unwrap_or_default(),
            frame_width: 0,
            frame_height: 0,
            frame_index: -1,
            unknown_chunk: RoqUnknownChunk::None,
            cinematic_lookahead: from_stream || options.end_policy == RoqEndPolicy::CinematicLookahead,
            silent: options.silent,
        };
        decoder.audio.setup_table();
        decoder.read_header(source)?;
        Ok(decoder)
    }

    fn read_header(&mut self, source: &str) -> Result<(), ClientError> {
        let header = self.stream.initialize()?;
        let mut reader = BinaryReader::new(&header, source);
        if self.stream.initialized_by_reset() {
            reader.skip(6).map_err(map_err)?;
        } else {
            if reader.u16().map_err(map_err)? != 0x1084 {
                return Err(ClientError::BadMedia(format!("{source}:0: invalid RoQ magic")));
            }
            reader.skip(4).map_err(map_err)?;
        }
        let rate = reader.u16().map_err(map_err)?;
        self.rate = if rate == 0 { 30 } else { u32::from(rate) };
        Ok(())
    }

    /// Decode from bytes.
    pub fn from_bytes(bytes: Vec<u8>, source: &str, options: RoqDecoderOptions) -> Result<Self, ClientError> {
        let policy = options.end_policy;
        let stream = RoqStream::from_bytes(bytes, source, policy);
        Self::bind(source, stream, options, false)
    }

    /// Decode from a stream.
    pub fn from_stream(stream: RoqStream, source: &str, options: RoqDecoderOptions) -> Result<Self, ClientError> {
        Self::bind(source, stream, options, true)
    }

    /// Rebind a fresh decoder over the retained stream (playback
    /// reset retains stream position and physical buffers).
    pub fn rebind(&mut self) -> Result<(), ClientError> {
        let source = self.stream.source().to_string();
        self.audio.setup_table();
        self.read_header(&source)?;
        self.frame_width = 0;
        self.frame_height = 0;
        self.frame_index = -1;
        self.unknown_chunk = RoqUnknownChunk::None;
        Ok(())
    }

    /// Capture a checkpoint.
    pub fn capture_checkpoint(&self) -> Result<RoqDecoderCheckpoint, ClientError> {
        Ok(RoqDecoderCheckpoint {
            rate: self.rate,
            width: self.frame_width,
            height: self.frame_height,
            frame_index: self.frame_index,
            unknown_chunk: self.unknown_chunk,
            cinematic_lookahead: self.cinematic_lookahead,
            silent: self.silent,
            scratch: self.scratch.capture_checkpoint(),
            stream: self.stream.capture_checkpoint()?,
        })
    }

    /// Restore a checkpoint.
    pub fn restore_checkpoint(&mut self, checkpoint: &RoqDecoderCheckpoint) -> Result<(), ClientError> {
        if checkpoint.cinematic_lookahead != self.cinematic_lookahead || checkpoint.silent != self.silent {
            return Err(ClientError::BadMedia("RoQ decoder mode differs".to_string()));
        }
        let (width, height) = (checkpoint.width, checkpoint.height);
        if (width == 0) != (height == 0)
            || !width.is_multiple_of(8)
            || !height.is_multiple_of(8)
            || width * height > 512 * 512
        {
            return Err(ClientError::BadMedia("invalid frame dimensions".to_string()));
        }
        if checkpoint.rate < 1 || checkpoint.frame_index < -1 {
            return Err(ClientError::BadMedia("invalid RoQ decoder cursor".to_string()));
        }
        self.rate = checkpoint.rate;
        self.frame_index = checkpoint.frame_index;
        self.unknown_chunk = checkpoint.unknown_chunk;
        self.scratch.restore_checkpoint(&checkpoint.scratch)?;
        self.stream.restore_checkpoint(&checkpoint.stream)?;
        self.frame_width = width;
        self.frame_height = height;
        Ok(())
    }

    /// Frame rate.
    #[must_use]
    pub const fn frame_rate(&self) -> u32 {
        self.rate
    }

    /// Frame width.
    #[must_use]
    pub const fn width(&self) -> usize {
        self.frame_width
    }

    /// Frame height.
    #[must_use]
    pub const fn height(&self) -> usize {
        self.frame_height
    }

    /// Whether the decoder sits inside a packet (`inPacket`).
    #[must_use]
    pub fn in_packet(&self) -> bool {
        self.stream.in_packet()
    }

    /// Whether an invalid lookahead was seen.
    #[must_use]
    pub fn has_invalid_lookahead(&self) -> bool {
        self.unknown_chunk != RoqUnknownChunk::None || self.stream.has_invalid_lookahead()
    }

    /// Whether the run ended cleanly (`resetAfterRun`).
    #[must_use]
    pub const fn reset_after_run(&self) -> bool {
        matches!(self.unknown_chunk, RoqUnknownChunk::RunTail)
    }

    /// Decoder scratch (presentation reads the live buffer).
    #[must_use]
    pub const fn scratch(&self) -> &RoqDecoderScratch {
        &self.scratch
    }

    /// Mutable scratch (playback reset clears it).
    pub fn scratch_mut(&mut self) -> &mut RoqDecoderScratch {
        &mut self.scratch
    }

    /// Close the backing stream.
    pub fn close_stream(&mut self) {
        self.stream.close();
    }

    /// Clear the retained stream buffer (the donor clears the shared
    /// file allocation through the scratch on reset).
    pub fn clear_stream_buffer(&mut self) {
        self.stream.clear_buffer();
    }

    /// `RoQReset` retains both physical image buffers and the
    /// codebooks between loops.
    pub fn rewind(&mut self) -> Result<(), ClientError> {
        self.stream.rewind()?;
        let source = self.stream.source().to_string();
        let header = self.stream.initialize()?;
        let mut reader = BinaryReader::new(&header, &source);
        reader.skip(6).map_err(map_err)?;
        let rate = reader.u16().map_err(map_err)?;
        self.rate = if rate == 0 { 30 } else { u32::from(rate) };
        self.frame_index = -1;
        self.unknown_chunk = RoqUnknownChunk::None;
        Ok(())
    }

    /// Decode the next frame/audio/end event (`next`).
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<RoqEvent, ClientError> {
        loop {
            match self.next_chunk(RoqChunkHooks::default())? {
                RoqChunkEvent::Frame(frame) => return Ok(RoqEvent::Frame(frame)),
                RoqChunkEvent::Audio(audio) => return Ok(RoqEvent::Audio(audio)),
                RoqChunkEvent::End => return Ok(RoqEvent::End),
                RoqChunkEvent::Info { .. } | RoqChunkEvent::Metadata => {}
            }
        }
    }

    /// Decode the next chunk (`nextChunk`).
    pub fn next_chunk(&mut self, mut hooks: RoqChunkHooks<'_>) -> Result<RoqChunkEvent, ClientError> {
        if self.unknown_chunk != RoqUnknownChunk::None {
            return Ok(RoqChunkEvent::End);
        }
        let Some(chunk) = self.stream.next_chunk()? else {
            return Ok(RoqChunkEvent::End);
        };
        let source = self.stream.source().to_string();
        let event = match chunk.id {
            0x1001 => {
                self.read_info(&chunk.bytes)?;
                if let Some(after) = hooks.after_info.as_deref_mut() {
                    after();
                }
                RoqChunkEvent::Info {
                    width: self.frame_width,
                    height: self.frame_height,
                }
            }
            0x1002 => {
                self.read_codebook(&chunk.bytes, chunk.flags)?;
                RoqChunkEvent::Metadata
            }
            0x1011 => RoqChunkEvent::Frame(self.read_frame(&chunk.bytes, &source, chunk.flags)?),
            // `ROQ_QUAD_JPEG` is a no-op in the original decoder.
            0x1012 | 0x1013 | 0x1030 => RoqChunkEvent::Metadata,
            0x1020 => {
                if self.silent {
                    RoqChunkEvent::Metadata
                } else {
                    RoqChunkEvent::Audio(self.read_audio(&chunk.bytes, &source, chunk.flags, 1)?)
                }
            }
            0x1021 => {
                if self.silent {
                    RoqChunkEvent::Metadata
                } else {
                    if let Some(before) = hooks.before_stereo.as_deref_mut() {
                        before();
                    }
                    RoqChunkEvent::Audio(self.read_audio(&chunk.bytes, &source, chunk.flags, 2)?)
                }
            }
            _ => {
                if !self.cinematic_lookahead {
                    return Err(ClientError::BadMedia(format!(
                        "{source}:{}: unsupported RoQ chunk 0x{:x}",
                        chunk.bytes.len(),
                        chunk.id
                    )));
                }
                RoqChunkEvent::End
            }
        };
        if let RoqChunkEvent::Audio(audio) = &event {
            if let Some(after) = hooks.after_audio.as_deref_mut() {
                after(audio);
            }
        }
        let ended = matches!(event, RoqChunkEvent::End);
        self.stream.complete_chunk(ended)?;
        if ended {
            self.unknown_chunk = if self.stream.has_invalid_lookahead() {
                RoqUnknownChunk::Interrupt
            } else {
                RoqUnknownChunk::RunTail
            };
        }
        Ok(event)
    }

    fn read_info(&mut self, payload: &[u8]) -> Result<(), ClientError> {
        if self.frame_index != -1 {
            // `RoQInterrupt` does not read repeated INFO payloads, and
            // retains `numQuads = 1`.
            if self.frame_index != 1 {
                self.frame_index = 0;
            }
            return Ok(());
        }
        let source = self.stream.source().to_string();
        let mut reader = BinaryReader::new(payload, &source);
        let width = reader.u16().map_err(map_err)?;
        let height = reader.u16().map_err(map_err)?;
        // Stored maxsize and minsize do not control the fixed 8/4
        // traversal.
        reader.u16().map_err(map_err)?;
        reader.u16().map_err(map_err)?;
        if width == 0
            || height == 0
            || !width.is_multiple_of(8)
            || !height.is_multiple_of(8)
            || u32::from(width) * u32::from(height) > 512 * 512
        {
            return Err(ClientError::BadMedia(format!(
                "{source}:{}: invalid RoQ quad dimensions",
                reader.offset()
            )));
        }
        self.frame_width = usize::from(width);
        self.frame_height = usize::from(height);
        self.frame_index = 0;
        Ok(())
    }

    fn read_codebook(&mut self, payload: &[u8], flags: u16) -> Result<(), ClientError> {
        let source = self.stream.source().to_string();
        let mut reader = BinaryReader::new(payload, &source);
        let profile = if self.cinematic_lookahead {
            RoqCodebookProfile::Source
        } else {
            RoqCodebookProfile::Diagnostic2x2
        };
        self.scratch.codebooks_mut().decode(
            &mut reader,
            flags,
            RoqCodebookMode::Normal,
            RoqCodebookFormat::Rgba,
            profile,
        )
    }

    fn read_audio(&self, payload: &[u8], source: &str, flags: u16, channels: u8) -> Result<RoqAudioEvent, ClientError> {
        if !payload.len().is_multiple_of(usize::from(channels)) {
            return Err(ClientError::BadMedia(format!(
                "{source}:{}: incomplete stereo sample pair",
                payload.len()
            )));
        }
        let count = payload.len();
        let mut samples = if channels == 1 {
            vec![0i16; count * 2]
        } else {
            vec![0i16; count]
        };
        if channels == 1 {
            self.audio
                .decode_mono_to_stereo(payload, &mut samples, count, false, flags)?;
        } else {
            self.audio
                .decode_stereo_to_stereo(payload, &mut samples, count, false, flags)?;
        }
        // `RoQInterrupt` submits `RllDecodeMonoToStereo`'s duplicated
        // buffer as mono.
        samples.truncate(count);
        Ok(RoqAudioEvent {
            samples,
            channels,
            sample_rate: ROQ_SAMPLE_RATE,
        })
    }

    fn front_offset(&self) -> usize {
        let byte_length = self.frame_width * self.frame_height * 4;
        if self.frame_index & 1 == 0 {
            0
        } else {
            byte_length
        }
    }

    fn back_offset(&self) -> usize {
        let byte_length = self.frame_width * self.frame_height * 4;
        if self.frame_index & 1 == 0 {
            byte_length
        } else {
            0
        }
    }

    fn read_frame(&mut self, payload: &[u8], source: &str, flags: u16) -> Result<RoqFrameEvent, ClientError> {
        if self.frame_width == 0 {
            return Err(ClientError::BadMedia(format!(
                "{source}:0: RoQ frame precedes quad info"
            )));
        }
        let mut vq = VqReader::new(payload, source);
        let (width, height) = (self.frame_width, self.frame_height);
        let mut y = 0;
        while y < height {
            let mut x = 0;
            while x < width {
                for quadrant in 0..4 {
                    let block_x = x + (quadrant & 1) * 8;
                    let block_y = y + (quadrant >> 1) * 8;
                    if block_x + 8 <= width && block_y + 8 <= height {
                        self.block(&mut vq, block_x, block_y, 8, flags)?;
                    }
                }
                x += 16;
            }
            y += 16;
        }
        // The source stops at the final quad; retail streams include
        // trailing control padding.
        let front = self.front_offset();
        let byte_length = width * height * 4;
        let rgba = self.scratch.frames[front..front + byte_length].to_vec();
        let event = RoqFrameEvent {
            rgba,
            index: self.frame_index,
            time: f64::from(self.frame_index) * 1000.0 / f64::from(self.rate),
        };
        if self.frame_index == 0 {
            let back = self.back_offset();
            self.scratch.frames.copy_within(front..front + byte_length, back);
        }
        self.frame_index += 1;
        Ok(event)
    }

    fn block(&mut self, vq: &mut VqReader<'_>, x: usize, y: usize, size: usize, flags: u16) -> Result<(), ClientError> {
        match vq.code()? {
            0 => Ok(()),
            1 => {
                let motion = vq.byte()?;
                let mean_x = i32::from((flags >> 8) as u8 as i8);
                let mean_y = i32::from((flags & 0xff) as u8 as i8);
                let scale = if self.frame_width == self.frame_height * 4 {
                    2
                } else {
                    1
                };
                let byte_length = self.frame_width * self.frame_height * 4;
                let reference = if self.frame_index & 1 == 0 {
                    byte_length as i64
                } else {
                    0
                };
                let stride = self.frame_width as i64;
                let offset = reference
                    + ((y as i64 + (8 - i64::from(motion & 15) - i64::from(mean_y)) * scale) * stride
                        + x as i64
                        + (8 - i64::from(motion >> 4) - i64::from(mean_x)) * scale)
                        * 4;
                let length = ((size as i64 - 1) * stride + size as i64) * 4;
                // Source motion addresses the full buffer, including
                // the current image and unused tail.
                if offset < 0 || offset + length > self.scratch.frames.len() as i64 {
                    return Err(ClientError::BadMedia(
                        "RoQ motion vector exceeds physical frame buffer".to_string(),
                    ));
                }
                let front = self.front_offset();
                let offset = offset as usize;
                for row in 0..size {
                    let mut pair = 0;
                    while pair < size * 4 {
                        for i in 0..8 {
                            let value = self.scratch.frames[offset + row * self.frame_width * 4 + pair + i];
                            self.scratch.frames[front + (y + row) * self.frame_width * 4 + x * 4 + pair + i] = value;
                        }
                        pair += 8;
                    }
                }
                Ok(())
            }
            2 => {
                let index = vq.byte()?;
                let book = if size == 8 { 8 } else { 4 };
                self.blit(book, index, size, x, y);
                Ok(())
            }
            3 => {
                for quadrant in 0..4 {
                    let next_x = x + (quadrant & 1) * (size / 2);
                    let next_y = y + (quadrant >> 1) * (size / 2);
                    if size == 8 {
                        self.block(vq, next_x, next_y, 4, flags)?;
                    } else {
                        let index = vq.byte()?;
                        self.blit(2, index, 2, next_x, next_y);
                    }
                }
                Ok(())
            }
            _ => Err(ClientError::BadMedia("invalid RoQ VQ code".to_string())),
        }
    }

    fn blit(&mut self, book: u8, index: u8, size: usize, x: usize, y: usize) {
        let front = self.front_offset();
        let width = self.frame_width;
        for row in 0..size {
            let source = (usize::from(index) * size * size + row * size) * 4;
            let destination = front + ((y + row) * width + x) * 4;
            let books = &self.scratch.codebooks;
            let table: &[u8] = match book {
                8 => books.book8(),
                4 => books.book4(),
                _ => books.book2(),
            };
            let mut pixels = [0u8; 32];
            pixels[..size * 4].copy_from_slice(&table[source..source + size * 4]);
            self.scratch.frames[destination..destination + size * 4].copy_from_slice(&pixels[..size * 4]);
        }
    }
}

/// A RoQ decoder checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct RoqDecoderCheckpoint {
    /// Frame rate.
    pub rate: u32,
    /// Frame width.
    pub width: usize,
    /// Frame height.
    pub height: usize,
    /// Frame index.
    pub frame_index: i32,
    /// Unknown-chunk state.
    pub unknown_chunk: RoqUnknownChunk,
    /// Cinematic lookahead.
    pub cinematic_lookahead: bool,
    /// Silent.
    pub silent: bool,
    /// Scratch.
    pub scratch: RoqScratchCheckpoint,
    /// Stream.
    pub stream: RoqStreamCheckpoint,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(id: u16, size: usize, flags: u16, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8; 8 + payload.len()];
        out[..2].copy_from_slice(&id.to_le_bytes());
        out[2..6].copy_from_slice(&(size as u32).to_le_bytes());
        out[6..8].copy_from_slice(&flags.to_le_bytes());
        out[8..].copy_from_slice(payload);
        out
    }

    fn canonical() -> Vec<u8> {
        [
            vec![0x84, 0x10, 0, 0, 0, 0, 30, 0],
            chunk(0x1001, 8, 0, &[8, 0, 8, 0, 0, 0, 0, 0]),
            chunk(0x1002, 10, 0x0101, &[16, 32, 48, 64, 100, 200, 0, 0, 0, 0]),
            chunk(0x1011, 3, 0, &[0x00, 0x80, 0x00]),
            chunk(0x1011, 3, 0, &[0x00, 0x40, 0x88]),
            chunk(0x1020, 4, 1000, &[0, 1, 2, 3]),
            chunk(0x1021, 4, 0x1234, &[1, 2, 3, 4]),
        ]
        .concat()
    }

    #[test]
    fn canonical_stream_matches_donor() {
        // Oracle events from the donor under bun.
        let mut decoder = RoqDecoder::from_bytes(canonical(), "<test>", RoqDecoderOptions::default()).unwrap();
        assert_eq!(decoder.frame_rate(), 30);
        assert!(matches!(
            decoder.next_chunk(RoqChunkHooks::default()).unwrap(),
            RoqChunkEvent::Info { width: 8, height: 8 }
        ));
        assert!(matches!(
            decoder.next_chunk(RoqChunkHooks::default()).unwrap(),
            RoqChunkEvent::Metadata
        ));
        let first = match decoder.next_chunk(RoqChunkHooks::default()).unwrap() {
            RoqChunkEvent::Frame(frame) => frame,
            event => panic!("expected frame, got {event:?}"),
        };
        assert_eq!(first.index, 0);
        assert_eq!(first.time, 0.0);
        assert_eq!(first.rgba.len(), 256);
        assert_eq!(first.rgba.iter().map(|byte| u32::from(*byte)).sum::<u32>(), 26128);
        assert_eq!(
            &first.rgba[..32],
            &[
                119, 0, 0, 255, 119, 0, 0, 255, 135, 0, 0, 255, 135, 0, 0, 255, 119, 0, 0, 255, 119, 0, 0, 255, 135, 0,
                0, 255, 135, 0, 0, 255
            ]
        );
        let second = match decoder.next_chunk(RoqChunkHooks::default()).unwrap() {
            RoqChunkEvent::Frame(frame) => frame,
            event => panic!("expected frame, got {event:?}"),
        };
        assert_eq!(second.index, 1);
        assert_eq!(second.time, 1000.0 / 30.0);
        assert_eq!(second.rgba, first.rgba);
        match decoder.next_chunk(RoqChunkHooks::default()).unwrap() {
            RoqChunkEvent::Audio(audio) => {
                assert_eq!(audio.channels, 1);
                assert_eq!(audio.sample_rate, ROQ_SAMPLE_RATE);
                assert_eq!(audio.samples, vec![1000, 1000, 1001, 1001]);
            }
            event => panic!("expected audio, got {event:?}"),
        }
        match decoder.next_chunk(RoqChunkHooks::default()).unwrap() {
            RoqChunkEvent::Audio(audio) => {
                assert_eq!(audio.channels, 2);
                assert_eq!(audio.samples, vec![4609, 13316, 4618, 13332]);
            }
            event => panic!("expected audio, got {event:?}"),
        }
        assert!(matches!(
            decoder.next_chunk(RoqChunkHooks::default()).unwrap(),
            RoqChunkEvent::End
        ));
        // Clean EOF sets no unknown-chunk state.
        assert!(!decoder.reset_after_run());
        assert!(!decoder.has_invalid_lookahead());
    }

    #[test]
    fn hooks_observe_decode() {
        let mut decoder = RoqDecoder::from_bytes(canonical(), "<test>", RoqDecoderOptions::default()).unwrap();
        let mut stereo = false;
        let mut infos = 0;
        let mut audio_events = 0;
        loop {
            let event = decoder
                .next_chunk(RoqChunkHooks {
                    before_stereo: Some(&mut || stereo = true),
                    after_info: Some(&mut || infos += 1),
                    after_audio: Some(&mut |_| audio_events += 1),
                })
                .unwrap();
            if matches!(event, RoqChunkEvent::End) {
                break;
            }
        }
        assert!(stereo);
        assert_eq!(infos, 1);
        assert_eq!(audio_events, 2);
    }

    #[test]
    fn silent_unknown_and_order_errors() {
        let mut decoder = RoqDecoder::from_bytes(
            canonical(),
            "<test>",
            RoqDecoderOptions {
                silent: true,
                ..RoqDecoderOptions::default()
            },
        )
        .unwrap();
        let mut audio = 0;
        loop {
            match decoder.next_chunk(RoqChunkHooks::default()).unwrap() {
                RoqChunkEvent::Audio(_) => audio += 1,
                RoqChunkEvent::End => break,
                _ => {}
            }
        }
        assert_eq!(audio, 0);

        // Unknown chunks are errors under the complete policy.
        let bytes = [
            vec![0x84, 0x10, 0, 0, 0, 0, 30, 0],
            chunk(0x1001, 8, 0, &[8, 0, 8, 0, 0, 0, 0, 0]),
            chunk(0x9999, 0, 0, &[]),
        ]
        .concat();
        let mut decoder = RoqDecoder::from_bytes(bytes, "<test>", RoqDecoderOptions::default()).unwrap();
        assert!(matches!(
            decoder.next_chunk(RoqChunkHooks::default()).unwrap(),
            RoqChunkEvent::Info { .. }
        ));
        assert!(decoder.next_chunk(RoqChunkHooks::default()).is_err());

        // Under the lookahead they end the run tail instead.
        let bytes = [
            vec![0x84, 0x10, 0, 0, 0, 0, 30, 0],
            chunk(0x1001, 8, 0, &[8, 0, 8, 0, 0, 0, 0, 0]),
            chunk(0x9999, 0, 0, &[]),
            chunk(0x1011, 2, 0, &[1, 2]),
        ]
        .concat();
        let mut decoder = RoqDecoder::from_bytes(
            bytes,
            "<test>",
            RoqDecoderOptions {
                end_policy: RoqEndPolicy::CinematicLookahead,
                ..RoqDecoderOptions::default()
            },
        )
        .unwrap();
        decoder.next_chunk(RoqChunkHooks::default()).unwrap();
        assert!(matches!(
            decoder.next_chunk(RoqChunkHooks::default()).unwrap(),
            RoqChunkEvent::End
        ));
        assert!(decoder.reset_after_run());

        // Frames before INFO are errors.
        let bytes = [vec![0x84, 0x10, 0, 0, 0, 0, 30, 0], chunk(0x1011, 3, 0, &[0, 0x80, 0])].concat();
        let mut decoder = RoqDecoder::from_bytes(bytes, "<test>", RoqDecoderOptions::default()).unwrap();
        assert!(decoder.next_chunk(RoqChunkHooks::default()).is_err());

        // Motion vectors cannot leave the physical buffer.
        let bytes = [
            vec![0x84, 0x10, 0, 0, 0, 0, 30, 0],
            chunk(0x1001, 8, 0, &[8, 0, 8, 0, 0, 0, 0, 0]),
            chunk(0x1011, 3, 0x7f00, &[0x00, 0x40, 0xff]),
        ]
        .concat();
        let mut decoder = RoqDecoder::from_bytes(bytes, "<test>", RoqDecoderOptions::default()).unwrap();
        decoder.next_chunk(RoqChunkHooks::default()).unwrap();
        assert!(decoder.next_chunk(RoqChunkHooks::default()).is_err());
    }

    #[test]
    fn subdivided_quads_and_repeated_info() {
        // Code 3 subdivides; NOP leaves black the decoder never blits.
        let bytes = [
            vec![0x84, 0x10, 0, 0, 0, 0, 30, 0],
            chunk(0x1001, 8, 0, &[8, 0, 8, 0, 0, 0, 0, 0]),
            chunk(0x1002, 10, 0x0101, &[16, 32, 48, 64, 100, 200, 0, 0, 0, 0]),
            chunk(0x1011, 2, 0, &[0x00, 0xC0]),
        ]
        .concat();
        let mut decoder = RoqDecoder::from_bytes(bytes, "<test>", RoqDecoderOptions::default()).unwrap();
        decoder.next_chunk(RoqChunkHooks::default()).unwrap();
        decoder.next_chunk(RoqChunkHooks::default()).unwrap();
        match decoder.next_chunk(RoqChunkHooks::default()).unwrap() {
            RoqChunkEvent::Frame(frame) => assert!(frame.rgba.iter().all(|byte| *byte == 0)),
            event => panic!("expected frame, got {event:?}"),
        }
        // Code 3 inside a 4x4 quad blits four 2x2 cells.
        let bytes = [
            vec![0x84, 0x10, 0, 0, 0, 0, 30, 0],
            chunk(0x1001, 8, 0, &[8, 0, 8, 0, 0, 0, 0, 0]),
            chunk(0x1002, 10, 0x0101, &[16, 32, 48, 64, 100, 200, 0, 0, 0, 0]),
            chunk(0x1011, 6, 0, &[0x00, 0xF0, 0, 0, 0, 0]),
        ]
        .concat();
        let mut decoder = RoqDecoder::from_bytes(bytes, "<test>", RoqDecoderOptions::default()).unwrap();
        decoder.next_chunk(RoqChunkHooks::default()).unwrap();
        decoder.next_chunk(RoqChunkHooks::default()).unwrap();
        match decoder.next_chunk(RoqChunkHooks::default()).unwrap() {
            RoqChunkEvent::Frame(frame) => {
                assert_eq!(&frame.rgba[..8], &[119, 0, 0, 255, 135, 0, 0, 255]);
            }
            event => panic!("expected frame, got {event:?}"),
        }
        // A repeated INFO retains `numQuads = 1`.
        let bytes = [
            vec![0x84, 0x10, 0, 0, 0, 0, 30, 0],
            chunk(0x1001, 8, 0, &[8, 0, 8, 0, 0, 0, 0, 0]),
            chunk(0x1002, 10, 0x0101, &[16, 32, 48, 64, 100, 200, 0, 0, 0, 0]),
            chunk(0x1011, 3, 0, &[0x00, 0x80, 0x00]),
            chunk(0x1001, 8, 0, &[8, 0, 8, 0, 0, 0, 0, 0]),
            chunk(0x1011, 3, 0, &[0x00, 0x40, 0x88]),
        ]
        .concat();
        let mut decoder = RoqDecoder::from_bytes(bytes, "<test>", RoqDecoderOptions::default()).unwrap();
        let mut indices = Vec::new();
        loop {
            match decoder.next_chunk(RoqChunkHooks::default()).unwrap() {
                RoqChunkEvent::Frame(frame) => indices.push(frame.index),
                RoqChunkEvent::End => break,
                _ => {}
            }
        }
        assert_eq!(indices, vec![0, 1]);
    }

    #[test]
    fn checkpoint_and_rewind_round_trip() {
        let mut decoder = RoqDecoder::from_bytes(canonical(), "<test>", RoqDecoderOptions::default()).unwrap();
        decoder.next_chunk(RoqChunkHooks::default()).unwrap();
        decoder.next_chunk(RoqChunkHooks::default()).unwrap();
        let checkpoint = decoder.capture_checkpoint().unwrap();
        let mut revived = RoqDecoder::from_bytes(canonical(), "<test>", RoqDecoderOptions::default()).unwrap();
        revived.restore_checkpoint(&checkpoint).unwrap();
        let (a, b) = (
            decoder.next_chunk(RoqChunkHooks::default()).unwrap(),
            revived.next_chunk(RoqChunkHooks::default()).unwrap(),
        );
        assert_eq!(a, b);
        revived.rewind().unwrap();
        assert_eq!(revived.width(), 8);
        assert!(matches!(
            revived.next_chunk(RoqChunkHooks::default()).unwrap(),
            RoqChunkEvent::Info { width: 8, height: 8 }
        ));
    }

    #[test]
    fn scratch_guards_ranges() {
        let mut scratch = RoqDecoderScratch::new();
        assert!(scratch.frame(0, 0).is_err());
        assert!(scratch.frame(ROQ_MAX_IMAGE + 1, 0).is_err());
        assert!(scratch.frame(16, 2).is_err());
        assert!(scratch.view(0, ROQ_PHYSICAL_FRAMES + 1).is_err());
        assert!(scratch.view(ROQ_PHYSICAL_FRAMES, 1).is_err());
        scratch.clear_movie_state();
        let checkpoint = scratch.capture_checkpoint();
        scratch.frames[0] = 9;
        scratch.restore_checkpoint(&checkpoint).unwrap();
        assert_eq!(scratch.frames[0], 0);
        let mut bad = checkpoint;
        bad.frames.push(0);
        assert!(scratch.restore_checkpoint(&bad).is_err());
    }
}
