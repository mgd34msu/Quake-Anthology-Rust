//! Cinematic containers: CIN, RoQ, Ogg framing.
//!
//! Donor provenance: `src/media/cin.ts` (headers, chunk walk,
//! `cinSampleRange`, `cinRgba`), `src/media/roq-stream.ts` (stream
//! framing, end policies, INFO validation), and `src/media/ogg.ts`
//! (RFC 3533 framing, Theora/Vorbis split).
//!
//! Container parsing only. Codec engines are deferred: CIN Huffman
//! bitstream decode, RoQ VQ/codebook/audio decode, Theora video and
//! Vorbis audio decode (need codec engines).

use qa_core::binary::{BinaryError, BinaryReader};

use super::source::{read_media, MediaInput};
use crate::ClientError;

fn map_err(source: &str, error: BinaryError) -> ClientError {
    ClientError::BadMedia(format!("{error} in {source}"))
}

// ---------------------------------------------------------------------------
// CIN
// ---------------------------------------------------------------------------

/// CIN audio format (`CinAudioFormat`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CinAudioFormat {
    /// Sample rate.
    pub sample_rate: i32,
    /// Channels.
    pub channels: u8,
    /// Sample bytes.
    pub sample_bytes: u8,
}

/// CIN header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CinHeader {
    /// Width.
    pub width: i32,
    /// Height.
    pub height: i32,
    /// Audio format.
    pub audio: Option<CinAudioFormat>,
}

/// CIN frame rate.
pub const CIN_FRAME_RATE: i32 = 14;

/// Parse a CIN header (`CinDecoder` header block).
pub fn parse_cin_header(input: &mut dyn MediaInput) -> Result<CinHeader, ClientError> {
    let source = input.source().to_string();
    if input.byte_length() < 20 + 65536 {
        return Err(ClientError::BadMedia(format!(
            "{source}:20: truncated media read of 65536 bytes"
        )));
    }
    let bytes = read_media(input, 0, 20)?;
    let mut reader = BinaryReader::new(&bytes, &source);
    let width = reader.i32().map_err(|error| map_err(&source, error))?;
    let height = reader.i32().map_err(|error| map_err(&source, error))?;
    let sample_rate = reader.i32().map_err(|error| map_err(&source, error))?;
    let sample_bytes = reader.i32().map_err(|error| map_err(&source, error))?;
    let channels = reader.i32().map_err(|error| map_err(&source, error))?;
    if width <= 0 || height <= 0 || width as i64 * height as i64 > 0x1000000 {
        return Err(ClientError::BadMedia(format!("{source}:0: invalid CIN dimensions")));
    }
    let audio = if sample_rate == 0 && sample_bytes == 0 && channels == 0 {
        None
    } else if sample_rate > 0
        && (sample_bytes == 1 || sample_bytes == 2)
        && (channels == 1 || channels == 2)
    {
        Some(CinAudioFormat {
            sample_rate,
            channels: channels as u8,
            sample_bytes: sample_bytes as u8,
        })
    } else {
        return Err(ClientError::BadMedia(format!("{source}:8: invalid CIN audio format")));
    };
    Ok(CinHeader {
        width,
        height,
        audio,
    })
}

/// A CIN chunk (`CinDecoder::next` framing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CinChunk {
    /// Palette update + frame payload.
    Frame {
        /// Palette present.
        palette: bool,
        /// Compressed size.
        size: usize,
        /// Payload offset.
        offset: usize,
    },
    /// End of stream.
    End,
}

/// Walk one CIN chunk header at an offset.
pub fn read_cin_chunk(
    input: &mut dyn MediaInput,
    offset: usize,
) -> Result<(CinChunk, usize), ClientError> {
    let source = input.source().to_string();
    let command_bytes = read_media(input, offset, 4)?;
    let mut reader = BinaryReader::new(&command_bytes, &source);
    let command = reader.i32().map_err(|error| map_err(&source, error))?;
    if command == 2 {
        return Ok((CinChunk::End, offset + 4));
    }
    let mut cursor = offset + 4;
    let palette = command == 1;
    if palette {
        cursor += 768;
        if cursor > input.byte_length() {
            return Err(ClientError::BadMedia(format!(
                "{source}:{cursor}: truncated media read of 768 bytes"
            )));
        }
    }
    let size_bytes = read_media(input, cursor, 4)?;
    let mut reader = BinaryReader::new(&size_bytes, &source);
    let size = reader.i32().map_err(|error| map_err(&source, error))?;
    if size < 4 || size > 0x20000 {
        return Err(ClientError::BadMedia(format!(
            "{source}:{cursor}: bad CIN compressed frame size"
        )));
    }
    Ok((
        CinChunk::Frame {
            palette,
            size: size as usize,
            offset: cursor + 4,
        },
        cursor + 4,
    ))
}

/// CIN sample range (`cinSampleRange`, C integer division).
pub fn cin_sample_range(frame: i64, sample_rate: i64) -> Result<(i64, i64), ClientError> {
    if frame < 0 || sample_rate < 0 || frame.checked_add(1).is_none() {
        return Err(ClientError::BadMedia("Invalid CIN sample position".to_string()));
    }
    (frame + 1)
        .checked_mul(sample_rate)
        .ok_or_else(|| ClientError::BadMedia("Invalid CIN sample position".to_string()))?;
    Ok((
        frame * sample_rate / CIN_FRAME_RATE as i64,
        (frame + 1) * sample_rate / CIN_FRAME_RATE as i64,
    ))
}

/// Expand indexed pixels through a 768-byte palette (`cinRgba`).
pub fn cin_rgba(pixels: &[u8], palette: &[u8]) -> Result<Vec<u8>, ClientError> {
    if palette.len() != 768 {
        return Err(ClientError::BadMedia("CIN palette requires 256 RGB colors".to_string()));
    }
    let mut rgba = vec![0u8; pixels.len() * 4];
    for (index, pixel) in pixels.iter().enumerate() {
        let base = usize::from(*pixel) * 3;
        rgba[index * 4] = palette[base];
        rgba[index * 4 + 1] = palette[base + 1];
        rgba[index * 4 + 2] = palette[base + 2];
        rgba[index * 4 + 3] = 255;
    }
    Ok(rgba)
}

// ---------------------------------------------------------------------------
// RoQ
// ---------------------------------------------------------------------------

/// RoQ magic.
pub const ROQ_MAGIC: u16 = 0x1084;
/// RoQ chunk: info.
pub const ROQ_INFO: u16 = 0x1001;
/// RoQ chunk: codebook.
pub const ROQ_CODEBOOK: u16 = 0x1002;
/// RoQ chunk: frame.
pub const ROQ_FRAME: u16 = 0x1011;
/// RoQ chunk: JPEG (decoder no-op).
pub const ROQ_QUAD_JPEG: u16 = 0x1012;
/// RoQ chunk: header-only marker.
pub const ROQ_HANG: u16 = 0x1013;
/// RoQ chunk: mono audio.
pub const ROQ_AUDIO_MONO: u16 = 0x1020;
/// RoQ chunk: stereo audio.
pub const ROQ_AUDIO_STEREO: u16 = 0x1021;
/// RoQ chunk: packet marker.
pub const ROQ_PACKET: u16 = 0x1030;

/// RoQ end policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoqEndPolicy {
    /// Complete reads; unknown chunks are errors.
    Complete,
    /// Cinematic lookahead; unknown chunks end the stream.
    CinematicLookahead,
}

/// RoQ file header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoqHeader {
    /// Frame rate (0 decodes to 30).
    pub frame_rate: u16,
}

/// Parse a RoQ header (`RoqDecoder` header block).
pub fn parse_roq_header(data: &[u8], source: &str) -> Result<(RoqHeader, usize), ClientError> {
    if data.len() < 8 {
        return Err(ClientError::BadMedia(format!("{source}:0: truncated RoQ header")));
    }
    let mut reader = BinaryReader::new(data, source);
    let magic = reader.u16().map_err(|error| map_err(source, error))?;
    if magic != ROQ_MAGIC {
        return Err(ClientError::BadMedia(format!("{source}:0: invalid RoQ magic")));
    }
    reader.skip(4).map_err(|error| map_err(source, error))?;
    let rate = reader.u16().map_err(|error| map_err(source, error))?;
    Ok((
        RoqHeader {
            frame_rate: if rate == 0 { 30 } else { rate },
        },
        8,
    ))
}

/// A RoQ chunk header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoqChunkHeader {
    /// Chunk id.
    pub id: u16,
    /// Payload size (three size bytes).
    pub size: usize,
    /// Flags.
    pub flags: u16,
}

/// Parse a RoQ chunk header (`parseHeader`).
pub fn parse_roq_chunk_header(data: &[u8], offset: usize, source: &str) -> Result<RoqChunkHeader, ClientError> {
    if offset + 8 > data.len() {
        return Err(ClientError::BadMedia(format!(
            "{source}:{offset}: truncated RoQ packet or lookahead header"
        )));
    }
    let mut reader = BinaryReader::new(&data[offset..offset + 8], source);
    let id = reader.u16().map_err(|error| map_err(source, error))?;
    let size = reader.u32().map_err(|error| map_err(source, error))? & 0x00ff_ffff;
    let flags = reader.u16().map_err(|error| map_err(source, error))?;
    Ok(RoqChunkHeader {
        id,
        size: size as usize,
        flags,
    })
}

/// A walked RoQ chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoqChunk {
    /// INFO with validated dimensions.
    Info {
        /// Width.
        width: u16,
        /// Height.
        height: u16,
    },
    /// Codebook metadata.
    Codebook,
    /// Frame payload.
    Frame {
        /// Payload offset.
        offset: usize,
        /// Payload size.
        size: usize,
        /// Flags.
        flags: u16,
    },
    /// Audio payload.
    Audio {
        /// Channels.
        channels: u8,
        /// Payload offset.
        offset: usize,
        /// Payload size.
        size: usize,
        /// Flags.
        flags: u16,
    },
    /// Header-only/metadata chunk.
    Metadata,
    /// End of stream.
    End,
}

/// Walk RoQ chunks (`RoqDecoder::nextChunk` framing, decode deferred).
pub fn walk_roq_chunks(
    data: &[u8],
    source: &str,
    policy: RoqEndPolicy,
) -> Result<Vec<RoqChunk>, ClientError> {
    let (_, mut offset) = parse_roq_header(data, source)?;
    let mut chunks = Vec::new();
    let mut saw_info = false;
    loop {
        if offset >= data.len() {
            chunks.push(RoqChunk::End);
            return Ok(chunks);
        }
        if offset + 8 > data.len() {
            if policy == RoqEndPolicy::CinematicLookahead {
                chunks.push(RoqChunk::End);
                return Ok(chunks);
            }
            return Err(ClientError::BadMedia(format!(
                "{source}:{offset}: truncated RoQ packet or lookahead header"
            )));
        }
        let header = parse_roq_chunk_header(data, offset, source)?;
        if header.size > 65536 {
            return Err(ClientError::BadMedia(format!(
                "{source}:{}: RoQ chunk exceeds 65536 bytes",
                offset + 2
            )));
        }
        // Header-only markers carry no payload.
        let decoded = matches!(header.id, ROQ_PACKET | ROQ_HANG);
        let payload = offset + 8;
        if !decoded && payload + header.size > data.len() {
            if policy == RoqEndPolicy::CinematicLookahead {
                chunks.push(RoqChunk::End);
                return Ok(chunks);
            }
            return Err(ClientError::BadMedia(format!(
                "{source}:{}: truncated RoQ payload",
                payload
            )));
        }
        match header.id {
            ROQ_INFO => {
                if payload + 8 > data.len() {
                    return Err(ClientError::BadMedia(format!(
                        "{source}:{payload}: truncated RoQ payload"
                    )));
                }
                let width = u16::from_le_bytes([data[payload], data[payload + 1]]);
                let height = u16::from_le_bytes([data[payload + 2], data[payload + 3]]);
                if !saw_info {
                    if width == 0
                        || height == 0
                        || width % 8 != 0
                        || height % 8 != 0
                        || u32::from(width) * u32::from(height) > 512 * 512
                    {
                        return Err(ClientError::BadMedia(format!(
                            "{source}:{payload}: invalid RoQ quad dimensions"
                        )));
                    }
                    saw_info = true;
                }
                chunks.push(RoqChunk::Info { width, height });
            }
            ROQ_CODEBOOK => chunks.push(RoqChunk::Codebook),
            ROQ_FRAME => {
                if !saw_info {
                    return Err(ClientError::BadMedia(format!(
                        "{source}:{payload}: RoQ frame precedes quad info"
                    )));
                }
                chunks.push(RoqChunk::Frame {
                    offset: payload,
                    size: header.size,
                    flags: header.flags,
                });
            }
            ROQ_QUAD_JPEG | ROQ_HANG | ROQ_PACKET => chunks.push(RoqChunk::Metadata),
            ROQ_AUDIO_MONO => chunks.push(RoqChunk::Audio {
                channels: 1,
                offset: payload,
                size: header.size,
                flags: header.flags,
            }),
            ROQ_AUDIO_STEREO => chunks.push(RoqChunk::Audio {
                channels: 2,
                offset: payload,
                size: header.size,
                flags: header.flags,
            }),
            _ => {
                if policy != RoqEndPolicy::CinematicLookahead {
                    return Err(ClientError::BadMedia(format!(
                        "{source}:{payload}: unsupported RoQ chunk 0x{:x}",
                        header.id
                    )));
                }
                chunks.push(RoqChunk::End);
                return Ok(chunks);
            }
        }
        offset = payload + if decoded { 0 } else { header.size };
    }
}

// ---------------------------------------------------------------------------
// Ogg
// ---------------------------------------------------------------------------

/// Maximum Ogg movie size (512 MiB).
pub const OGG_MAX_MOVIE: usize = 512 * 1024 * 1024;
/// Maximum Ogg packet size (16 MiB).
pub const OGG_MAX_PACKET: usize = 16 * 1024 * 1024;

/// An Ogg packet (`OggPacket`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OggPacket {
    /// Data.
    pub data: Vec<u8>,
    /// First in stream.
    pub first: bool,
    /// Last in stream.
    pub last: bool,
    /// Granule position.
    pub granule: i64,
    /// Index.
    pub index: usize,
}

/// An Ogg movie (`OggMovie`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OggMovie {
    /// Theora video packets.
    pub video: Vec<OggPacket>,
    /// Vorbis pages (or `None`).
    pub audio: Option<Vec<u8>>,
}

fn checksum_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    for (index, slot) in table.iter_mut().enumerate() {
        let mut value = (index as u32) << 24;
        for _ in 0..8 {
            value = if value & 0x8000_0000 != 0 {
                (value << 1) ^ 0x04c1_1db7
            } else {
                value << 1
            };
        }
        *slot = value;
    }
    table
}

struct LogicalStream {
    sequence: u32,
    packets: Vec<OggPacket>,
    pages: Vec<(usize, usize)>,
    pieces: Vec<(usize, usize)>,
    pending: usize,
    ended: bool,
}

/// Decode Ogg framing (`decodeOggMovie`).
pub fn decode_ogg_movie(bytes: &[u8]) -> Result<OggMovie, ClientError> {
    if bytes.len() > OGG_MAX_MOVIE {
        return Err(ClientError::BadMedia("Ogg movie exceeds 512 MiB".to_string()));
    }
    let table = checksum_table();
    let mut streams: Vec<(u32, LogicalStream)> = Vec::new();
    let mut offset = 0usize;
    let invalid = |message: &str| ClientError::BadMedia(message.to_string());
    while offset < bytes.len() {
        if offset + 27 > bytes.len()
            || bytes[offset..offset + 4] != [b'O', b'g', b'g', b'S']
            || bytes[offset + 4] != 0
        {
            return Err(invalid("Invalid or truncated Ogg page"));
        }
        let flags = bytes[offset + 5];
        let segments = bytes[offset + 26] as usize;
        if flags & !7 != 0 || offset + 27 + segments > bytes.len() {
            return Err(invalid("Invalid Ogg page flags/lacing"));
        }
        let lacing = &bytes[offset + 27..offset + 27 + segments];
        let payload_start = offset + 27 + segments;
        let payload_bytes: usize = lacing.iter().map(|length| *length as usize).sum();
        let end = payload_start + payload_bytes;
        if end > bytes.len() {
            return Err(invalid("Truncated Ogg page payload"));
        }
        let mut crc = 0u32;
        for position in offset..end {
            let byte = if (offset + 22..offset + 26).contains(&position) {
                0
            } else {
                bytes[position]
            };
            crc = (crc << 8) ^ table[((crc >> 24) ^ u32::from(byte)) as usize & 255];
        }
        let stored = u32::from_le_bytes([
            bytes[offset + 22],
            bytes[offset + 23],
            bytes[offset + 24],
            bytes[offset + 25],
        ]);
        if crc != stored {
            return Err(invalid("Ogg page checksum mismatch"));
        }
        let serial = u32::from_le_bytes([
            bytes[offset + 14],
            bytes[offset + 15],
            bytes[offset + 16],
            bytes[offset + 17],
        ]);
        let sequence = u32::from_le_bytes([
            bytes[offset + 18],
            bytes[offset + 19],
            bytes[offset + 20],
            bytes[offset + 21],
        ]);
        let stream_index = streams.iter().position(|(serial_number, _)| *serial_number == serial);
        let stream_index = match stream_index {
            Some(index) => {
                let (_, stream) = &streams[index];
                if flags & 2 != 0 || stream.ended {
                    return Err(invalid("Ogg logical stream restarted after admission"));
                }
                index
            }
            None => {
                if flags & 2 == 0 || sequence != 0 || streams.len() >= 8 {
                    return Err(invalid("Invalid Ogg logical stream start"));
                }
                streams.push((
                    serial,
                    LogicalStream {
                        sequence: 0,
                        packets: Vec::new(),
                        pages: Vec::new(),
                        pieces: Vec::new(),
                        pending: 0,
                        ended: false,
                    },
                ));
                streams.len() - 1
            }
        };
        let (_, stream) = &mut streams[stream_index];
        if sequence != stream.sequence || (flags & 1 != 0) != !stream.pieces.is_empty() {
            return Err(invalid("Ogg packet sequence discontinuity"));
        }
        stream.sequence += 1;
        stream.pages.push((offset, end));
        let mut cursor = payload_start;
        let mut completed: Vec<usize> = Vec::new();
        for length in lacing {
            let length = *length as usize;
            stream.pieces.push((cursor, cursor + length));
            stream.pending += length;
            cursor += length;
            if stream.pending > OGG_MAX_PACKET {
                return Err(invalid("Ogg packet exceeds 16 MiB"));
            }
            if length < 255 {
                let mut data = Vec::with_capacity(stream.pending);
                for (start, end) in &stream.pieces {
                    data.extend_from_slice(&bytes[*start..*end]);
                }
                let index = stream.packets.len();
                stream.packets.push(OggPacket {
                    data,
                    first: index == 0,
                    last: false,
                    granule: -1,
                    index,
                });
                completed.push(index);
                stream.pieces.clear();
                stream.pending = 0;
            }
        }
        if let Some(last) = completed.last() {
            let granule = i64::from_le_bytes([
                bytes[offset + 6],
                bytes[offset + 7],
                bytes[offset + 8],
                bytes[offset + 9],
                bytes[offset + 10],
                bytes[offset + 11],
                bytes[offset + 12],
                bytes[offset + 13],
            ]);
            let packet = stream.packets[*last].clone();
            stream.packets[*last] = OggPacket {
                granule,
                last: flags & 4 != 0,
                ..packet
            };
        }
        if flags & 4 != 0 {
            if !stream.pieces.is_empty() {
                return Err(invalid("Ogg stream ends inside a packet"));
            }
            stream.ended = true;
        }
        offset = end;
    }
    let mut video: Option<Vec<OggPacket>> = None;
    let mut audio: Option<Vec<u8>> = None;
    for (_, stream) in &streams {
        if !stream.ended {
            return Err(invalid("Truncated Ogg logical stream"));
        }
        let Some(first) = stream.packets.first() else {
            return Err(invalid("Empty Ogg logical stream"));
        };
        let signature = String::from_utf8_lossy(first.data.get(1..7).unwrap_or(&[])).into_owned();
        if first.data.first() == Some(&0x80) && signature == "theora" {
            if video.is_some() {
                return Err(invalid("Multiple Theora video streams are unsupported"));
            }
            video = Some(stream.packets.clone());
        } else if first.data.first() == Some(&1) && signature == "vorbis" {
            if audio.is_some() {
                return Err(invalid("Multiple Vorbis soundtracks are unsupported"));
            }
            let mut joined = Vec::new();
            for (start, end) in &stream.pages {
                joined.extend_from_slice(&bytes[*start..*end]);
            }
            audio = Some(joined);
        }
    }
    let Some(video) = video else {
        return Err(invalid("Ogg movie has no complete Theora video"));
    };
    if video.len() < 4 {
        return Err(invalid("Ogg movie has no complete Theora video"));
    }
    Ok(OggMovie { video, audio })
}

/// Theora identification (`TheoraDecoder` header surface).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TheoraIdent {
    /// Picture width.
    pub width: usize,
    /// Picture height.
    pub height: usize,
    /// Frame interval in milliseconds.
    pub frame_ms: f64,
}

/// Parse a Theora identification header (container-level fields).
pub fn parse_theora_ident(packet: &OggPacket) -> Result<TheoraIdent, ClientError> {
    let data = &packet.data;
    if data.len() < 42
        || data[0] != 0x80
        || data[1..7] != *b"theora"
        || data[7] != 3
        || data[8] != 2
    {
        return Err(ClientError::BadMedia("Invalid Theora identification header".to_string()));
    }
    let width =
        ((u32::from(data[14]) << 16) | (u32::from(data[15]) << 8) | u32::from(data[16])) as usize;
    let height =
        ((u32::from(data[17]) << 16) | (u32::from(data[18]) << 8) | u32::from(data[19])) as usize;
    let numerator = u32::from_be_bytes([data[22], data[23], data[24], data[25]]) as f64;
    let denominator = u32::from_be_bytes([data[26], data[27], data[28], data[29]]) as f64;
    if width == 0 || height == 0 || numerator <= 0.0 || denominator <= 0.0 {
        return Err(ClientError::BadMedia("Invalid Theora picture size or frame rate".to_string()));
    }
    Ok(TheoraIdent {
        width,
        height,
        frame_ms: 1000.0 * denominator / numerator,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::source::MemMedia;

    fn cin_bytes() -> Vec<u8> {
        let mut bytes = vec![0u8; 20 + 65536 + 8];
        bytes[0..4].copy_from_slice(&64i32.to_le_bytes());
        bytes[4..8].copy_from_slice(&64i32.to_le_bytes());
        // command 2 (end) after the Huffman table.
        bytes[20 + 65536..20 + 65536 + 4].copy_from_slice(&2i32.to_le_bytes());
        bytes
    }

    #[test]
    fn cin_header_parses() {
        let mut input = MemMedia::new(cin_bytes(), "<test>");
        let header = parse_cin_header(&mut input).unwrap();
        assert_eq!((header.width, header.height), (64, 64));
        assert!(header.audio.is_none());
        let (chunk, _) = read_cin_chunk(&mut input, 20 + 65536).unwrap();
        assert_eq!(chunk, CinChunk::End);
    }

    #[test]
    fn cin_sample_ranges_divide() {
        assert_eq!(cin_sample_range(0, 22050).unwrap(), (0, 1575));
        assert_eq!(cin_sample_range(1, 22050).unwrap(), (1575, 3150));
        assert!(cin_sample_range(-1, 22050).is_err());
    }

    #[test]
    fn cin_rgba_expands() {
        let palette: Vec<u8> = (0..768).map(|index| (index % 256) as u8).collect();
        let rgba = cin_rgba(&[0, 1], &palette).unwrap();
        assert_eq!(rgba.len(), 8);
        assert_eq!(&rgba[0..4], &[0, 1, 2, 255]);
    }

    #[test]
    fn roq_header_and_info_walk() {
        let mut data = vec![0u8; 8 + 8 + 8];
        data[0..2].copy_from_slice(&ROQ_MAGIC.to_le_bytes());
        data[6..8].copy_from_slice(&0u16.to_le_bytes());
        data[8..10].copy_from_slice(&ROQ_INFO.to_le_bytes());
        data[10..14].copy_from_slice(&8u32.to_le_bytes());
        data[16..18].copy_from_slice(&64u16.to_le_bytes());
        data[18..20].copy_from_slice(&64u16.to_le_bytes());
        let chunks = walk_roq_chunks(&data, "<test>", RoqEndPolicy::Complete).unwrap();
        assert!(matches!(chunks[0], RoqChunk::Info { width: 64, height: 64 }));
        assert_eq!(*chunks.last().unwrap(), RoqChunk::End);
    }

    #[test]
    fn roq_frame_before_info_is_an_error() {
        let mut data = vec![0u8; 8 + 8 + 4];
        data[0..2].copy_from_slice(&ROQ_MAGIC.to_le_bytes());
        data[6..8].copy_from_slice(&30u16.to_le_bytes());
        data[8..10].copy_from_slice(&ROQ_FRAME.to_le_bytes());
        data[10..14].copy_from_slice(&4u32.to_le_bytes());
        assert!(walk_roq_chunks(&data, "<test>", RoqEndPolicy::Complete).is_err());
    }

    #[test]
    fn ogg_rejects_garbage() {
        assert!(decode_ogg_movie(&[0u8; 64]).is_err());
        assert!(decode_ogg_movie(&[]).is_err());
    }
}
