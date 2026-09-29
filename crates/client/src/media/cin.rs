//! CIN decoder: order-1 Huffman frames and PCM audio.
//!
//! Donor provenance: `src/media/cin.ts` (Quake II `cl_cin.c`,
//! Copyright (C) 1997-2001 Id Software, Inc. GPL-2.0-or-later).
//!
//! Sample ranges and palette expansion live in
//! [`crate::media::containers`] (`cinSampleRange`, `cinRgba`) and are
//! reused here.

use qa_core::binary::{BinaryError, BinaryReader};

use super::containers::{cin_rgba, cin_sample_range, CinAudioFormat, CIN_FRAME_RATE};
use super::source::{read_media, MediaInput, MemMedia};
use super::types::AudioSamples;
use crate::ClientError;

fn map_err(error: BinaryError) -> ClientError {
    ClientError::BadMedia(error.to_string())
}

/// CIN audio samples (`CinAudio`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CinAudio {
    /// Samples.
    pub samples: AudioSamples,
    /// Channels.
    pub channels: u8,
    /// Sample rate.
    pub sample_rate: u32,
    /// Source sample.
    pub source_sample: usize,
}

/// A decoded CIN frame (`CinFrame`).
#[derive(Debug, Clone, PartialEq)]
pub struct CinFrame {
    /// Frame index.
    pub index: usize,
    /// Source time in milliseconds.
    pub time: f64,
    /// Indexed pixels.
    pub pixels: Vec<u8>,
    /// Palette snapshot.
    pub palette: Vec<u8>,
    /// Audio.
    pub audio: Option<CinAudio>,
}

/// Order-1 Huffman decoder (`CinHuffman`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CinHuffman {
    nodes: Vec<i32>,
    roots: Vec<i32>,
    source: String,
}

impl CinHuffman {
    /// Build tables from 65536 counts.
    pub fn new(counts: &[u8], source: &str) -> Result<Self, ClientError> {
        if counts.len() != 65536 {
            return Err(ClientError::BadMedia(format!(
                "{source}:0: CIN requires 65536 Huffman counts"
            )));
        }
        let mut nodes = vec![0i32; 256 * 256 * 2];
        let mut roots = vec![0i32; 256];
        let mut weights = [0i32; 512];
        let mut used = [0u8; 512];
        for context in 0..256 {
            weights.fill(0);
            used.fill(0);
            for (slot, count) in weights
                .iter_mut()
                .zip(counts[context * 256..(context + 1) * 256].iter())
            {
                *slot = i32::from(*count);
            }
            let mut count = 256;
            while count != 511 {
                let left = smallest(&weights, &mut used, count);
                if left < 0 {
                    break;
                }
                let right = smallest(&weights, &mut used, count);
                if right < 0 {
                    break;
                }
                let offset = context * 512 + (count - 256) * 2;
                nodes[offset] = left;
                nodes[offset + 1] = right;
                weights[count] = weights[left as usize] + weights[right as usize];
                count += 1;
            }
            roots[context] = (count - 1) as i32;
        }
        Ok(Self {
            nodes,
            roots,
            source: source.to_string(),
        })
    }

    /// Decode a compressed frame.
    pub fn decode(&self, compressed: &[u8], expected_pixels: usize) -> Result<Vec<u8>, ClientError> {
        let mut reader = BinaryReader::new(compressed, &self.source);
        let count = reader.i32().map_err(map_err)?;
        if count < 0 || count as usize != expected_pixels {
            return Err(ClientError::BadMedia(format!(
                "{}:0: CIN frame has {count} pixels, expected {expected_pixels}",
                self.source
            )));
        }
        let count = count as usize;
        let mut output = vec![0u8; count];
        let mut context = 0usize;
        let mut value = 0u8;
        let mut bits = 0u8;
        for slot in output.iter_mut() {
            let mut node = self.roots[context];
            while node >= 256 {
                if bits == 0 {
                    value = reader.u8().map_err(map_err)?;
                    bits = 8;
                }
                let child = self.nodes[context * 512 + (node as usize - 256) * 2 + usize::from(value & 1)];
                if child < 0 || child >= node {
                    return Err(ClientError::BadMedia(format!(
                        "{}:{}: invalid CIN Huffman branch",
                        self.source,
                        reader.offset()
                    )));
                }
                node = child;
                value >>= 1;
                bits -= 1;
            }
            *slot = node as u8;
            context = node as usize;
        }
        Ok(output)
    }
}

fn smallest(weights: &[i32; 512], used: &mut [u8; 512], limit: usize) -> i32 {
    let mut best = 99_999_999;
    let mut selected = -1;
    for index in 0..limit {
        let count = weights[index];
        if used[index] == 0 && count != 0 && count < best {
            best = count;
            selected = index as i32;
        }
    }
    if selected != -1 {
        used[selected as usize] = 1;
    }
    selected
}

/// A CIN decoder (`CinDecoder`).
pub struct CinDecoder {
    input: Box<dyn MediaInput>,
    width: usize,
    height: usize,
    audio_format: Option<CinAudioFormat>,
    huffman: CinHuffman,
    colors: [u8; 768],
    offset: usize,
    frame_index: usize,
    ended: bool,
    closed: bool,
}

impl CinDecoder {
    fn bind(mut input: Box<dyn MediaInput>) -> Result<Self, ClientError> {
        let source = input.source().to_string();
        let header = read_media(&mut *input, 0, 20)?;
        let mut reader = BinaryReader::new(&header, &source);
        let width = reader.i32().map_err(map_err)?;
        let height = reader.i32().map_err(map_err)?;
        let sample_rate = reader.i32().map_err(map_err)?;
        let sample_bytes = reader.i32().map_err(map_err)?;
        let channels = reader.i32().map_err(map_err)?;
        if width <= 0 || height <= 0 || i64::from(width) * i64::from(height) > 0x1000000 {
            return Err(ClientError::BadMedia(format!("{source}:0: invalid CIN dimensions")));
        }
        let audio_format = if sample_rate == 0 && sample_bytes == 0 && channels == 0 {
            None
        } else if sample_rate > 0 && (sample_bytes == 1 || sample_bytes == 2) && (channels == 1 || channels == 2) {
            Some(CinAudioFormat {
                sample_rate,
                channels: channels as u8,
                sample_bytes: sample_bytes as u8,
            })
        } else {
            return Err(ClientError::BadMedia(format!("{source}:8: invalid CIN audio format")));
        };
        let counts = read_media(&mut *input, 20, 65536)?;
        let huffman = CinHuffman::new(&counts, &source)?;
        Ok(Self {
            input,
            width: width as usize,
            height: height as usize,
            audio_format,
            huffman,
            colors: [0; 768],
            offset: 20 + 65536,
            frame_index: 0,
            ended: false,
            closed: false,
        })
    }

    /// Decode from bytes.
    pub fn from_bytes(bytes: Vec<u8>, source: &str) -> Result<Self, ClientError> {
        Self::bind(Box::new(MemMedia::new(bytes, source)))
    }

    /// Decode from an input.
    pub fn from_input(input: Box<dyn MediaInput>) -> Result<Self, ClientError> {
        Self::bind(input)
    }

    /// Width.
    #[must_use]
    pub const fn width(&self) -> usize {
        self.width
    }

    /// Height.
    #[must_use]
    pub const fn height(&self) -> usize {
        self.height
    }

    /// Frame rate (`14`).
    #[must_use]
    pub const fn frame_rate(&self) -> i32 {
        CIN_FRAME_RATE
    }

    /// Audio format.
    #[must_use]
    pub const fn audio_format(&self) -> Option<CinAudioFormat> {
        self.audio_format
    }

    /// Current palette.
    #[must_use]
    pub fn palette(&self) -> Vec<u8> {
        self.colors.to_vec()
    }

    /// Next frame index.
    #[must_use]
    pub const fn next_frame_index(&self) -> usize {
        self.frame_index
    }

    /// Capture a checkpoint.
    #[must_use]
    pub fn capture_checkpoint(&self) -> CinDecoderCheckpoint {
        CinDecoderCheckpoint {
            width: self.width,
            height: self.height,
            input_length: self.input.byte_length(),
            offset: self.offset,
            frame_index: self.frame_index,
            ended: self.ended,
            closed: self.closed,
            palette: self.colors.to_vec(),
        }
    }

    /// Restore a checkpoint.
    pub fn restore_checkpoint(&mut self, checkpoint: &CinDecoderCheckpoint) -> Result<(), ClientError> {
        if checkpoint.width != self.width
            || checkpoint.height != self.height
            || checkpoint.input_length != self.input.byte_length()
        {
            return Err(ClientError::BadMedia("CIN identity differs".to_string()));
        }
        if checkpoint.offset < 20 + 65536
            || checkpoint.offset > self.input.byte_length()
            || checkpoint.palette.len() != self.colors.len()
        {
            return Err(ClientError::BadMedia("invalid CIN cursor or palette".to_string()));
        }
        self.frame_index = checkpoint.frame_index;
        self.ended = checkpoint.ended;
        self.offset = checkpoint.offset;
        self.colors.copy_from_slice(&checkpoint.palette);
        if checkpoint.closed {
            self.close();
        }
        Ok(())
    }

    fn read(&mut self, length: usize) -> Result<Vec<u8>, ClientError> {
        let bytes = read_media(&mut *self.input, self.offset, length)?;
        self.offset += length;
        Ok(bytes)
    }

    /// Decode the next frame (`next`).
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<CinNext, ClientError> {
        if self.closed {
            return Err(ClientError::BadMedia("CIN decoder is closed".to_string()));
        }
        if self.ended {
            return Ok(CinNext::End);
        }
        let source = self.input.source().to_string();
        let command_bytes = self.read(4)?;
        let mut reader = BinaryReader::new(&command_bytes, &source);
        let command = reader.i32().map_err(map_err)?;
        if command == 2 {
            self.ended = true;
            return Ok(CinNext::End);
        }
        if command == 1 {
            let palette = self.read(768)?;
            self.colors.copy_from_slice(&palette);
        }
        let size_bytes = self.read(4)?;
        let mut reader = BinaryReader::new(&size_bytes, &source);
        let size = reader.i32().map_err(map_err)?;
        if !(4..=0x20000).contains(&size) {
            return Err(ClientError::BadMedia(format!(
                "{}:{}: bad CIN compressed frame size",
                source,
                self.offset - 4
            )));
        }
        let compressed = self.read(size as usize)?;
        let mut audio = None;
        if let Some(format) = self.audio_format {
            let (start, end) = cin_sample_range(self.frame_index as i64, i64::from(format.sample_rate))?;
            let bytes =
                self.read((end - start) as usize * usize::from(format.sample_bytes) * usize::from(format.channels))?;
            let samples = if format.sample_bytes == 2 {
                let mut reader = BinaryReader::new(&bytes, &source);
                let mut signed = Vec::with_capacity(bytes.len() / 2);
                for _ in 0..bytes.len() / 2 {
                    signed.push(reader.i16().map_err(map_err)?);
                }
                AudioSamples::I16(signed)
            } else {
                AudioSamples::U8(bytes)
            };
            audio = Some(CinAudio {
                samples,
                channels: format.channels,
                sample_rate: format.sample_rate as u32,
                source_sample: start as usize,
            });
        }
        let pixels = self.huffman.decode(&compressed, self.width * self.height)?;
        let index = self.frame_index;
        self.frame_index += 1;
        Ok(CinNext::Frame(CinFrame {
            index,
            time: index as f64 * 1000.0 / f64::from(CIN_FRAME_RATE),
            pixels,
            palette: self.palette(),
            audio,
        }))
    }

    /// Rewind the decoder.
    pub fn rewind(&mut self) -> Result<(), ClientError> {
        if self.closed {
            return Err(ClientError::BadMedia("CIN decoder is closed".to_string()));
        }
        self.offset = 20 + 65536;
        self.frame_index = 0;
        self.ended = false;
        self.colors.fill(0);
        Ok(())
    }

    /// Close the decoder.
    pub fn close(&mut self) {
        self.closed = true;
    }

    /// Expand indexed pixels through a palette (`cinRgba`).
    pub fn rgba(pixels: &[u8], palette: &[u8]) -> Result<Vec<u8>, ClientError> {
        cin_rgba(pixels, palette)
    }
}

/// A CIN decoder step.
#[derive(Debug, Clone, PartialEq)]
pub enum CinNext {
    /// Decoded frame.
    Frame(CinFrame),
    /// End of stream.
    End,
}

/// A CIN decoder checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CinDecoderCheckpoint {
    /// Width.
    pub width: usize,
    /// Height.
    pub height: usize,
    /// Input length.
    pub input_length: usize,
    /// Read offset.
    pub offset: usize,
    /// Frame index.
    pub frame_index: usize,
    /// Ended.
    pub ended: bool,
    /// Closed.
    pub closed: bool,
    /// Palette.
    pub palette: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_bytes() -> Vec<u8> {
        let mut header = vec![0u8; 20];
        header[..4].copy_from_slice(&4i32.to_le_bytes());
        header[4..8].copy_from_slice(&4i32.to_le_bytes());
        header[8..12].copy_from_slice(&14i32.to_le_bytes());
        header[12..16].copy_from_slice(&1i32.to_le_bytes());
        header[16..20].copy_from_slice(&1i32.to_le_bytes());
        let counts = vec![0u8; 65536];
        let palette: Vec<u8> = (0..768).map(|index| (index % 256) as u8).collect();
        let compressed = 16i32.to_le_bytes().to_vec();
        let mut bytes = [header, counts].concat();
        bytes.extend_from_slice(&1i32.to_le_bytes());
        bytes.extend_from_slice(&palette);
        bytes.extend_from_slice(&4i32.to_le_bytes());
        bytes.extend_from_slice(&compressed);
        bytes.push(200);
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&4i32.to_le_bytes());
        bytes.extend_from_slice(&compressed);
        bytes.push(100);
        bytes.extend_from_slice(&2i32.to_le_bytes());
        bytes
    }

    #[test]
    fn huffman_matches_donor() {
        // Oracle vectors from the donor under bun.
        let trivial = CinHuffman::new(&vec![0u8; 65536], "<test>").unwrap();
        assert_eq!(trivial.decode(&16i32.to_le_bytes(), 16).unwrap(), vec![255u8; 16]);
        let mut counts = vec![0u8; 65536];
        counts[0] = 1;
        counts[1] = 1;
        counts[256] = 1;
        counts[257] = 1;
        let bits = CinHuffman::new(&counts, "<test>").unwrap();
        assert_eq!(
            bits.decode(&[16, 0, 0, 0, 0b1011_0001, 0b0101_0101], 16).unwrap(),
            vec![1, 0, 0, 0, 1, 1, 0, 1, 1, 0, 1, 0, 1, 0, 1, 0]
        );
        assert!(CinHuffman::new(&[0u8; 10], "<test>").is_err());
        assert!(trivial.decode(&15i32.to_le_bytes(), 16).is_err());
    }

    #[test]
    fn decoder_matches_donor() {
        let mut decoder = CinDecoder::from_bytes(frame_bytes(), "<test>").unwrap();
        assert_eq!((decoder.width(), decoder.height()), (4, 4));
        assert_eq!(decoder.frame_rate(), 14);
        assert_eq!(
            decoder.audio_format(),
            Some(CinAudioFormat {
                sample_rate: 14,
                channels: 1,
                sample_bytes: 1,
            })
        );
        match decoder.next().unwrap() {
            CinNext::Frame(frame) => {
                assert_eq!(frame.index, 0);
                assert_eq!(frame.time, 0.0);
                assert_eq!(frame.pixels, vec![255u8; 16]);
                let audio = frame.audio.unwrap();
                assert_eq!(audio.samples, AudioSamples::U8(vec![200]));
                assert_eq!(audio.source_sample, 0);
                assert_eq!(&frame.palette[..4], &[0, 1, 2, 3]);
            }
            CinNext::End => panic!("expected frame"),
        }
        match decoder.next().unwrap() {
            CinNext::Frame(frame) => {
                assert_eq!(frame.index, 1);
                assert_eq!(frame.time, 1000.0 / 14.0);
                let audio = frame.audio.unwrap();
                assert_eq!(audio.samples, AudioSamples::U8(vec![100]));
                assert_eq!(audio.source_sample, 1);
            }
            CinNext::End => panic!("expected frame"),
        }
        assert_eq!(decoder.next().unwrap(), CinNext::End);
        decoder.rewind().unwrap();
        assert!(matches!(decoder.next().unwrap(), CinNext::Frame(_)));
    }

    #[test]
    fn sixteen_bit_audio_decodes() {
        let mut header = vec![0u8; 20];
        header[..4].copy_from_slice(&4i32.to_le_bytes());
        header[4..8].copy_from_slice(&4i32.to_le_bytes());
        header[8..12].copy_from_slice(&14i32.to_le_bytes());
        header[12..16].copy_from_slice(&2i32.to_le_bytes());
        header[16..20].copy_from_slice(&1i32.to_le_bytes());
        let mut bytes = [header, vec![0u8; 65536]].concat();
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&4i32.to_le_bytes());
        bytes.extend_from_slice(&16i32.to_le_bytes());
        bytes.extend_from_slice(&0x1234i16.to_le_bytes());
        bytes.extend_from_slice(&2i32.to_le_bytes());
        let mut decoder = CinDecoder::from_bytes(bytes, "<test>").unwrap();
        match decoder.next().unwrap() {
            CinNext::Frame(frame) => {
                let audio = frame.audio.unwrap();
                assert_eq!(audio.samples, AudioSamples::I16(vec![0x1234]));
            }
            CinNext::End => panic!("expected frame"),
        }
    }

    #[test]
    fn headers_and_checkpoints() {
        let mut bad = frame_bytes();
        bad[..4].copy_from_slice(&0i32.to_le_bytes());
        assert!(CinDecoder::from_bytes(bad, "<test>").is_err());
        let mut bad = frame_bytes();
        bad[8..12].copy_from_slice(&(-1i32).to_le_bytes());
        assert!(CinDecoder::from_bytes(bad, "<test>").is_err());

        let mut decoder = CinDecoder::from_bytes(frame_bytes(), "<test>").unwrap();
        decoder.next().unwrap();
        let checkpoint = decoder.capture_checkpoint();
        let mut revived = CinDecoder::from_bytes(frame_bytes(), "<test>").unwrap();
        revived.restore_checkpoint(&checkpoint).unwrap();
        assert_eq!(revived.next().unwrap(), decoder.next().unwrap());
        let mut bad = checkpoint.clone();
        bad.palette.push(0);
        assert!(revived.restore_checkpoint(&bad).is_err());
        revived.close();
        assert!(revived.next().is_err());
        assert_eq!(
            CinDecoder::rgba(&[0, 1], &(0..768).map(|index| (index % 256) as u8).collect::<Vec<_>>()).unwrap(),
            vec![0, 1, 2, 255, 3, 4, 5, 255]
        );
    }
}
