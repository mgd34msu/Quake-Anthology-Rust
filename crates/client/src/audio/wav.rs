//! WAV decoding.
//!
//! Donor provenance: `src/audio/wav.ts` (`readWavInfo`, `decodeWav`,
//! `decodeQ3Wav`, `decodeQuakeWav`, from `GetWavinfo`/`ResampleSfx`).

use qa_core::binary::{BinaryError, BinaryReader};

use super::error::AudioError;

/// Decoded PCM sound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PcmSound {
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Channel count (1 or 2).
    pub channels: u8,
    /// Interleaved samples.
    pub samples: Vec<i16>,
    /// Frame count.
    pub frame_count: usize,
    /// WAV loop marker, if any.
    pub loop_start: Option<usize>,
}

/// Decoded WAV plus its source width.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedWav {
    /// Decoded PCM.
    pub pcm: PcmSound,
    /// Source bytes per sample (1, 2, or 3).
    pub source_bytes_per_sample: u8,
}

fn reject(source: &str, offset: usize, message: impl Into<String>) -> AudioError {
    AudioError::Binary(BinaryError::custom(source, offset, message).to_string())
}

fn four_cc(reader: &mut BinaryReader) -> Result<String, AudioError> {
    let mut name = String::with_capacity(4);
    for _ in 0..4 {
        name.push(char::from(reader.u8().map_err(AudioError::from)?));
    }
    Ok(name)
}

fn expect_four_cc(reader: &mut BinaryReader, source: &str, expected: &str) -> Result<(), AudioError> {
    let offset = reader.offset();
    let actual = four_cc(reader)?;
    if actual != expected {
        return Err(reject(source, offset, format!("expected {expected:?}, got {actual:?}")));
    }
    Ok(())
}

fn matches_four_cc(reader: &mut BinaryReader, offset: usize, expected: &str) -> Result<bool, AudioError> {
    reader.seek(offset).map_err(AudioError::from)?;
    for (index, byte) in expected.bytes().enumerate() {
        let _ = index;
        if reader.u8().map_err(AudioError::from)? != byte {
            return Ok(false);
        }
    }
    Ok(true)
}

struct ChunkRange {
    offset: usize,
    length: usize,
}

fn find_chunk(reader: &mut BinaryReader, source: &str, start: usize, length: usize, name: &str) -> Result<Option<ChunkRange>, AudioError> {
    let mut offset = start;
    while offset < length {
        if offset + 8 > reader.length() {
            return Err(reject(source, offset + 4, "truncated WAV chunk header"));
        }
        reader.seek(offset + 4).map_err(AudioError::from)?;
        let chunk_length = reader.i32().map_err(AudioError::from)?;
        if chunk_length < 0 {
            return Ok(None);
        }
        if chunk_length == 0x7fff_ffff {
            return Err(reject(source, offset + 4, "WAV chunk alignment overflows a signed int"));
        }
        if matches_four_cc(reader, offset, name)? {
            return Ok(Some(ChunkRange {
                offset: offset + 8,
                length: chunk_length as usize,
            }));
        }
        offset += 8 + (((chunk_length as usize) + 1) & !1);
    }
    Ok(None)
}

fn pcm_sample(reader: &mut BinaryReader, data_offset: usize, bytes_per_sample: u8, index: usize) -> Result<i32, AudioError> {
    reader
        .seek(data_offset + index * if bytes_per_sample == 2 { 2 } else { 1 })
        .map_err(AudioError::from)?;
    if bytes_per_sample == 2 {
        Ok(i32::from(reader.i16().map_err(AudioError::from)?))
    } else {
        Ok((i32::from(reader.u8().map_err(AudioError::from)?) - 128) << 8)
    }
}

fn pcm_samples(reader: &mut BinaryReader, data_offset: usize, bytes_per_sample: u8, sample_count: usize) -> Result<Vec<i16>, AudioError> {
    let mut samples = Vec::with_capacity(sample_count);
    for index in 0..sample_count {
        samples.push(pcm_sample(reader, data_offset, bytes_per_sample, index)? as i16);
    }
    Ok(samples)
}

/// Lazy WAV info over borrowed bytes (`readWavInfo`).
#[derive(Debug, Clone, Copy)]
pub struct WavInfo<'a> {
    data: &'a [u8],
    source: &'a str,
    /// Sample rate in Hz (0 when the header is missing).
    pub sample_rate: u32,
    /// Channel count (0 when the header is missing).
    pub channels: u32,
    /// Source bytes per sample.
    pub source_bytes_per_sample: u32,
    /// Frame count.
    pub frame_count: usize,
    data_offset: usize,
}

impl WavInfo<'_> {
    /// Read the reached `ResampleSfx` index.
    pub fn sample(&self, index: usize) -> Result<i32, AudioError> {
        let mut reader = BinaryReader::new(self.data, self.source);
        pcm_sample(&mut reader, self.data_offset, self.source_bytes_per_sample as u8, index)
    }

    /// Materialize mono PCM while the file allocation is live.
    pub fn decode(&self) -> Result<PcmSound, AudioError> {
        if self.channels != 1 {
            return Err(reject(self.source, self.data_offset, "source WAV sound decoding requires mono channels"));
        }
        let mut reader = BinaryReader::new(self.data, self.source);
        let samples = pcm_samples(&mut reader, self.data_offset, self.source_bytes_per_sample as u8, self.frame_count)?;
        Ok(PcmSound {
            sample_rate: self.sample_rate,
            channels: 1,
            samples,
            frame_count: self.frame_count,
            loop_start: None,
        })
    }
}

/// Read WAV info, printing ordinary diagnostics (`readWavInfo`).
pub fn read_wav_info<'a>(bytes: &'a [u8], length: usize, source: &'a str, print: &mut dyn FnMut(&str)) -> Result<WavInfo<'a>, AudioError> {
    let mut reader = BinaryReader::new(bytes, source);
    if length > reader.length() {
        return Err(reject(source, 0, format!("WAV file length {length} exceeds its physical allocation")));
    }
    let (mut sample_rate, mut channels, mut source_bytes_per_sample, mut frame_count, mut data_offset) = (0u32, 0u32, 0u32, 0usize, 0usize);
    macro_rules! info {
        () => {
            WavInfo {
                data: bytes,
                source,
                sample_rate,
                channels,
                source_bytes_per_sample,
                frame_count,
                data_offset,
            }
        };
    }
    let riff = find_chunk(&mut reader, source, 0, length, "RIFF")?;
    let wave = match &riff {
        None => false,
        Some(riff) => matches_four_cc(&mut reader, riff.offset, "WAVE")?,
    };
    if !wave {
        print("Missing RIFF/WAVE chunks\n");
        return Ok(info!());
    }
    let chunk_start = riff.map_or(0, |riff| riff.offset + 4);
    let format = find_chunk(&mut reader, source, chunk_start, length, "fmt ")?;
    let Some(format) = format else {
        print("Missing fmt chunk\n");
        return Ok(info!());
    };
    reader.seek(format.offset).map_err(AudioError::from)?;
    let encoding = reader.i16().map_err(AudioError::from)?;
    channels = reader.i16().map_err(AudioError::from)? as u32;
    sample_rate = reader.i32().map_err(AudioError::from)? as u32;
    reader.skip(6).map_err(AudioError::from)?;
    source_bytes_per_sample = (reader.i16().map_err(AudioError::from)? as i32 / 8) as u32;
    if encoding != 1 {
        print("Microsoft PCM format only\n");
        return Ok(info!());
    }
    let data = find_chunk(&mut reader, source, chunk_start, length, "data")?;
    let Some(data) = data else {
        print("Missing data chunk\n");
        return Ok(info!());
    };
    if source_bytes_per_sample == 0 {
        return Err(reject(source, data.offset - 4, "WAV sample count divides by zero source width"));
    }
    frame_count = data.length / source_bytes_per_sample as usize;
    data_offset = data.offset;
    Ok(info!())
}

struct WavFormat {
    sample_rate: u32,
    channels: u8,
    bytes_per_sample: u8,
    block_align: usize,
}

fn parse_format(reader: &mut BinaryReader, source: &str, chunk_offset: usize) -> Result<WavFormat, AudioError> {
    if reader.length() < 16 {
        return Err(reject(source, chunk_offset, format!("fmt chunk is {} bytes, expected at least 16", reader.length())));
    }
    let encoding = reader.u16().map_err(AudioError::from)?;
    if encoding != 1 {
        return Err(reject(source, chunk_offset, format!("unsupported WAV encoding {encoding}; only PCM is supported")));
    }
    let channels = reader.u16().map_err(AudioError::from)?;
    if channels != 1 && channels != 2 {
        return Err(reject(source, chunk_offset + 2, format!("unsupported WAV channel count {channels}")));
    }
    let sample_rate = reader.u32().map_err(AudioError::from)?;
    if sample_rate == 0 {
        return Err(reject(source, chunk_offset + 4, "WAV sample rate must be positive"));
    }
    let byte_rate = reader.u32().map_err(AudioError::from)?;
    let block_align = reader.u16().map_err(AudioError::from)? as usize;
    let bits_per_sample = reader.u16().map_err(AudioError::from)?;
    if bits_per_sample != 8 && bits_per_sample != 16 && bits_per_sample != 24 {
        return Err(reject(
            source,
            chunk_offset + 14,
            format!("unsupported WAV sample width {bits_per_sample} bits"),
        ));
    }
    let bytes_per_sample = if bits_per_sample == 8 { 1 } else if bits_per_sample == 16 { 2 } else { 3 };
    let expected_block_align = channels as usize * bytes_per_sample as usize;
    if block_align != expected_block_align {
        return Err(reject(
            source,
            chunk_offset + 12,
            format!("WAV block alignment {block_align} does not match {expected_block_align}"),
        ));
    }
    let expected_byte_rate = sample_rate as u64 * block_align as u64;
    if u64::from(byte_rate) != expected_byte_rate {
        return Err(reject(
            source,
            chunk_offset + 8,
            format!("WAV byte rate {byte_rate} does not match {expected_byte_rate}"),
        ));
    }
    Ok(WavFormat {
        sample_rate,
        channels: channels as u8,
        bytes_per_sample,
        block_align,
    })
}

fn parse_cue_loop(reader: &mut BinaryReader, source: &str, chunk_offset: usize) -> Result<Option<u32>, AudioError> {
    if reader.length() < 4 {
        return Err(reject(source, chunk_offset, "truncated WAV cue chunk"));
    }
    let cue_count = reader.u32().map_err(AudioError::from)? as usize;
    if 4 + cue_count * 24 > reader.length() {
        return Err(reject(source, chunk_offset, "truncated WAV cue-point records"));
    }
    if cue_count == 0 {
        return Ok(None);
    }
    reader.skip(20).map_err(AudioError::from)?;
    Ok(Some(reader.u32().map_err(AudioError::from)?))
}

fn parse_sampler_loop(reader: &mut BinaryReader, source: &str, chunk_offset: usize, source_sentinel: bool) -> Result<Option<u32>, AudioError> {
    if reader.length() < 36 {
        return Err(reject(source, chunk_offset, "truncated WAV sampler chunk"));
    }
    reader.seek(28).map_err(AudioError::from)?;
    let loop_count = reader.u32().map_err(AudioError::from)? as usize;
    if 36 + loop_count * 24 > reader.length() {
        return Err(reject(source, chunk_offset, "truncated WAV sampler-loop records"));
    }
    if loop_count == 0 {
        return Ok(None);
    }
    reader.seek(40).map_err(AudioError::from)?;
    let kind = reader.u32().map_err(AudioError::from)?;
    let start = reader.u32().map_err(AudioError::from)?;
    let end = reader.u32().map_err(AudioError::from)?;
    if source_sentinel && kind == 255 && start == 0xffff_ffff && end == 0xffff_ffff {
        return Ok(None);
    }
    Ok(Some(start))
}

fn decode_pcm(bytes: &[u8], source: &str, source_signed_chunks: bool, read_loops: bool) -> Result<DecodedWav, AudioError> {
    let mut reader = BinaryReader::new(bytes, source);
    if reader.length() < 12 {
        return Err(reject(source, 0, "truncated RIFF/WAVE header"));
    }
    expect_four_cc(&mut reader, source, "RIFF")?;
    let riff_size = reader.u32().map_err(AudioError::from)? as usize;
    if riff_size < 4 {
        return Err(reject(source, 4, format!("invalid RIFF size {riff_size}")));
    }
    let riff_end = 8 + riff_size;
    if riff_end > reader.length() {
        return Err(reject(source, 4, format!("RIFF size {riff_size} exceeds {}-byte input", reader.length())));
    }
    expect_four_cc(&mut reader, source, "WAVE")?;
    let mut format: Option<WavFormat> = None;
    let mut data: Option<ChunkRange> = None;
    let mut cue_loop_start: Option<u32> = None;
    let mut sampler_loop_start: Option<u32> = None;
    while reader.offset() < riff_end {
        let chunk_header_offset = reader.offset();
        if riff_end - chunk_header_offset < 8 {
            return Err(reject(source, chunk_header_offset, "truncated WAV chunk header"));
        }
        let chunk_id = four_cc(&mut reader)?;
        let chunk_length = reader.u32().map_err(AudioError::from)? as usize;
        if source_signed_chunks && chunk_length > 0x7fff_ffff {
            break;
        }
        let chunk_offset = reader.offset();
        if chunk_length > riff_end || chunk_offset > riff_end - chunk_length {
            if source_signed_chunks
                && format.is_some()
                && data.is_some()
                && chunk_id == "LIST"
                && riff_end - chunk_offset >= 4
                && matches_four_cc(&mut reader, chunk_offset, "INFO")?
            {
                break;
            }
            return Err(reject(source, chunk_header_offset + 4, format!("WAV chunk {chunk_id:?} exceeds RIFF bounds")));
        }
        let chunk_end = chunk_offset + chunk_length;
        let next_chunk_offset = if chunk_end < riff_end { chunk_end + (chunk_length & 1) } else { chunk_end };
        if chunk_id == "fmt " && format.is_none() {
            let mut section = reader.section(chunk_offset, chunk_length).map_err(AudioError::from)?;
            format = Some(parse_format(&mut section, source, chunk_offset)?);
        } else if chunk_id == "data" && data.is_none() {
            data = Some(ChunkRange { offset: chunk_offset, length: chunk_length });
        } else if read_loops && chunk_id == "cue " && cue_loop_start.is_none() {
            let mut section = reader.section(chunk_offset, chunk_length).map_err(AudioError::from)?;
            cue_loop_start = parse_cue_loop(&mut section, source, chunk_offset)?;
        } else if read_loops && chunk_id == "smpl" && sampler_loop_start.is_none() {
            let mut section = reader.section(chunk_offset, chunk_length).map_err(AudioError::from)?;
            sampler_loop_start = parse_sampler_loop(&mut section, source, chunk_offset, source_signed_chunks)?;
        }
        reader.seek(next_chunk_offset).map_err(AudioError::from)?;
    }
    let format = format.ok_or_else(|| reject(source, 12, "missing WAV fmt chunk"))?;
    let data = data.ok_or_else(|| reject(source, 12, "missing WAV data chunk"))?;
    if data.length % format.block_align != 0 {
        return Err(reject(
            source,
            data.offset,
            format!("WAV data length {} is not a multiple of block alignment {}", data.length, format.block_align),
        ));
    }
    let frame_count = data.length / format.block_align;
    let sample_count = frame_count * usize::from(format.channels);
    let loop_start = cue_loop_start.or(sampler_loop_start);
    let mut samples = vec![0i16; sample_count];
    if format.bytes_per_sample == 3 {
        for (index, sample) in samples.iter_mut().enumerate() {
            reader.seek(data.offset + index * 3 + 1).map_err(AudioError::from)?;
            *sample = reader.i16().map_err(AudioError::from)?;
        }
    } else {
        samples = pcm_samples(&mut reader, data.offset, format.bytes_per_sample, sample_count)?;
    }
    if loop_start.is_some_and(|marker| marker as usize >= frame_count) {
        return Err(reject(
            source,
            data.offset,
            format!("WAV loop start {} is outside {frame_count} frames", loop_start.unwrap_or(0)),
        ));
    }
    Ok(DecodedWav {
        pcm: PcmSound {
            sample_rate: format.sample_rate,
            channels: format.channels,
            samples,
            frame_count,
            loop_start: loop_start.map(|marker| marker as usize),
        },
        source_bytes_per_sample: format.bytes_per_sample,
    })
}

/// Asset inspection across the complete RIFF (`decodeWav`).
pub fn decode_wav(bytes: &[u8], source: &str) -> Result<DecodedWav, AudioError> {
    decode_pcm(bytes, source, false, true)
}

/// Q3 fmt/data decode without loop control (`decodeQ3Wav`).
pub fn decode_q3_wav(bytes: &[u8], source: &str) -> Result<DecodedWav, AudioError> {
    decode_pcm(bytes, source, true, false)
}

/// Quake/Q2 decode with Sound Forge loop-end trimming (`decodeQuakeWav`).
pub fn decode_quake_wav(bytes: &[u8], source: &str) -> Result<DecodedWav, AudioError> {
    let pcm = decode_pcm(bytes, source, true, true)?;
    let Some(marker) = pcm.pcm.loop_start else {
        return Ok(pcm);
    };
    let mut reader = BinaryReader::new(bytes, source);
    let mut cue_seen = false;
    let mut loop_end: Option<usize> = None;
    reader.seek(12).map_err(AudioError::from)?;
    let riff_size = u32::from_le_bytes(bytes[4..8].try_into().unwrap_or([0; 4])) as usize;
    let riff_end = 8 + riff_size;
    while reader.offset() + 8 <= riff_end {
        let name = four_cc(&mut reader)?;
        let length = reader.u32().map_err(AudioError::from)? as usize;
        let start = reader.offset();
        if length > 0x7fff_ffff {
            break;
        }
        if (length > riff_end || start > riff_end - length) && name == "LIST" && riff_end - start >= 4 && matches_four_cc(&mut reader, start, "INFO")? {
            break;
        }
        if name == "cue " {
            cue_seen = true;
        }
        if cue_seen && name == "LIST" && length >= 24 {
            let mut list = reader.section(start, length).map_err(AudioError::from)?;
            list.seek(20).map_err(AudioError::from)?;
            if four_cc(&mut list)? == "mark" {
                list.seek(16).map_err(AudioError::from)?;
                loop_end = Some(marker + list.u32().map_err(AudioError::from)? as usize);
                break;
            }
        }
        reader.seek(riff_end.min(start + length + (length & 1))).map_err(AudioError::from)?;
    }
    let Some(loop_end) = loop_end else {
        return Ok(pcm);
    };
    if loop_end <= marker || loop_end > pcm.pcm.frame_count {
        return Err(reject(source, reader.offset(), "Sound Forge loop end outside WAV frames"));
    }
    let mut trimmed = pcm;
    trimmed.pcm.samples.truncate(loop_end * usize::from(trimmed.pcm.channels));
    trimmed.pcm.frame_count = loop_end;
    Ok(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&44u32.to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&11025u32.to_le_bytes());
        bytes.extend_from_slice(&22050u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&8u32.to_le_bytes());
        bytes.extend_from_slice(&1000i16.to_le_bytes());
        bytes.extend_from_slice(&(-1000i16).to_le_bytes());
        bytes.extend_from_slice(&2000i16.to_le_bytes());
        bytes.extend_from_slice(&(-2000i16).to_le_bytes());
        bytes
    }

    #[test]
    fn decodes_pcm_with_loops() {
        let bytes = wav_bytes();
        let decoded = decode_wav(&bytes, "test").unwrap();
        assert_eq!(decoded.pcm.sample_rate, 11025);
        assert_eq!(decoded.pcm.channels, 1);
        assert_eq!(decoded.pcm.samples, vec![1000, -1000, 2000, -2000]);
        assert_eq!(decoded.pcm.frame_count, 4);
        assert_eq!(decoded.pcm.loop_start, None);
        assert_eq!(decoded.source_bytes_per_sample, 2);
        let quake = decode_quake_wav(&bytes, "test").unwrap();
        assert_eq!(quake.pcm.samples.len(), 4);
        assert!(decode_wav(b"RIFF", "short").is_err());
    }

    #[test]
    fn info_reports_but_continues() {
        let bytes = wav_bytes();
        let mut printed = Vec::new();
        let info = read_wav_info(&bytes, bytes.len(), "test", &mut |text| printed.push(text.to_string())).unwrap();
        assert_eq!(info.sample_rate, 11025);
        assert_eq!(info.frame_count, 4);
        assert_eq!(info.sample(0).unwrap(), 1000);
        assert_eq!(info.decode().unwrap().samples, vec![1000, -1000, 2000, -2000]);
        assert!(printed.is_empty());
        let missing = read_wav_info(b"NOPEWAVEFMT", 11, "missing", &mut |text| printed.push(text.to_string())).unwrap();
        assert_eq!(missing.sample_rate, 0);
        assert!(!printed.is_empty());
    }
}
