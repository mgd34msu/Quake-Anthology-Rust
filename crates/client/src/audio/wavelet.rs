//! Daubechies wavelet and mu-law voice compression.
//!
//! Donor provenance: `src/audio/wavelet.ts` (`daub4`, `wt1`,
//! `muLawEncode`, `muLawDecode`, `SourceWaveletCodec`, from
//! `snd_wavelet.c`).

use super::error::AudioError;

/// Shorts per wavelet chunk.
pub const WAVELET_CHUNK_SAMPLES: usize = 1024;
/// Bytes per wavelet chunk.
pub const WAVELET_CHUNK_BYTES: usize = WAVELET_CHUNK_SAMPLES * 2;

const C0: f64 = 0.4829629131445341;
const C1: f64 = 0.8365163037378079;
const C2: f64 = 0.2241438680420134;
const C3: f64 = -0.1294095225512604;

/// One compressed chunk; bytes and shorts share the allocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaveletSoundChunk {
    /// Shared chunk storage.
    pub data: [u8; WAVELET_CHUNK_BYTES],
    /// Next chunk.
    pub next: Option<Box<WaveletSoundChunk>>,
    /// Written bytes.
    pub size: usize,
}

impl WaveletSoundChunk {
    /// Chunk over caller storage.
    #[must_use]
    pub const fn new(data: [u8; WAVELET_CHUNK_BYTES]) -> Self {
        Self { data, next: None, size: 0 }
    }
}

/// Compressed sound.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WaveletSound {
    /// Sample count.
    pub sound_length: usize,
    /// First chunk.
    pub sound_data: Option<Box<WaveletSoundChunk>>,
}

fn integer(value: i64, minimum: i64, maximum: i64, name: &str) -> Result<(), AudioError> {
    if value < minimum || value > maximum {
        return Err(AudioError::IntRange {
            name: name.to_string(),
            minimum,
            maximum,
        });
    }
    Ok(())
}

fn transform_input(samples: &[f32], size: usize, sign: i32) -> Result<(), AudioError> {
    integer(size as i64, 0, samples.len().min(4096) as i64, "wavelet size")?;
    integer(i64::from(sign), -2_147_483_648, 2_147_483_647, "wavelet sign")
}

/// Daubechies 4-tap transform step.
pub fn daub4(samples: &mut [f32], size: usize, sign: i32) -> Result<(), AudioError> {
    transform_input(samples, size, sign)?;
    if size < 4 {
        return Ok(());
    }
    if !size.is_multiple_of(2) {
        return Err(AudioError::WaveletOdd);
    }
    let read = |samples: &[f32], index: usize| -> Result<f64, AudioError> {
        samples.get(index).copied().map(f64::from).ok_or(AudioError::WaveletRead(index))
    };
    let mut scratch = vec![0f32; size];
    let half = size >> 1;
    if sign >= 0 {
        let mut i = 0;
        let mut j = 0;
        while j <= size - 4 {
            scratch[i] = (C0 * read(samples, j)? + C1 * read(samples, j + 1)? + C2 * read(samples, j + 2)? + C3 * read(samples, j + 3)?) as f32;
            scratch[i + half] = (C3 * read(samples, j)? - C2 * read(samples, j + 1)? + C1 * read(samples, j + 2)? - C0 * read(samples, j + 3)?) as f32;
            j += 2;
            i += 1;
        }
        scratch[i] = (C0 * read(samples, size - 2)? + C1 * read(samples, size - 1)? + C2 * read(samples, 0)? + C3 * read(samples, 1)?) as f32;
        scratch[i + half] = (C3 * read(samples, size - 2)? - C2 * read(samples, size - 1)? + C1 * read(samples, 0)? - C0 * read(samples, 1)?) as f32;
    } else {
        scratch[0] = (C2 * read(samples, half - 1)? + C1 * read(samples, size - 1)? + C0 * read(samples, 0)? + C3 * read(samples, half)?) as f32;
        scratch[1] = (C3 * read(samples, half - 1)? - C0 * read(samples, size - 1)? + C1 * read(samples, 0)? - C2 * read(samples, half)?) as f32;
        let mut j = 2;
        for i in 0..half - 1 {
            scratch[j] = (C2 * read(samples, i)? + C1 * read(samples, i + half)? + C0 * read(samples, i + 1)? + C3 * read(samples, i + half + 1)?) as f32;
            j += 1;
            scratch[j] = (C3 * read(samples, i)? - C0 * read(samples, i + half)? + C1 * read(samples, i + 1)? - C2 * read(samples, i + half + 1)?) as f32;
            j += 1;
        }
    }
    samples[..size].copy_from_slice(&scratch);
    Ok(())
}

/// Wavelet transform across stages (`wt1`).
pub fn wt1(samples: &mut [f32], size: usize, sign: i32) -> Result<(), AudioError> {
    transform_input(samples, size, sign)?;
    let inverse_start_length = size / 4;
    if inverse_start_length == 0 {
        return Err(AudioError::WaveletSmall);
    }
    if sign >= 0 {
        let mut length = size;
        while length >= inverse_start_length {
            daub4(samples, length, sign)?;
            length >>= 1;
        }
    } else {
        let mut length = inverse_start_length;
        while length <= size {
            daub4(samples, length, sign)?;
            length <<= 1;
        }
    }
    Ok(())
}

/// Mu-law encode one sample.
pub fn mu_law_encode(sample: i32) -> Result<u8, AudioError> {
    integer(i64::from(sample), -32768, 32767, "mu-law sample")?;
    let sign = if sample < 0 { 0u32 } else { 0x80u32 };
    let adjusted = (sample.unsigned_abs() + 132).min(32_767);
    let exponent = 31 - ((adjusted >> 7) & 0xff).leading_zeros();
    let mantissa = (adjusted >> (exponent + 3)) & 0xf;
    Ok((!(sign | (exponent << 4) | mantissa) & 0xff) as u8)
}

/// Mu-law decode one byte.
pub fn mu_law_decode(value: u8) -> i16 {
    let law = (!u32::from(value)) & 0xff;
    let exponent = (law >> 4) & 0x7;
    let mantissa = (law & 0xf) + 16;
    let adjusted = ((mantissa << (exponent + 3)) as i32) - 132;
    if law & 0x80 != 0 { adjusted as i16 } else { (-adjusted) as i16 }
}

/// Voice codec with the lazy mu-law table.
pub struct SourceWaveletCodec {
    allocate_chunk: Box<dyn FnMut() -> WaveletSoundChunk>,
    mulaw_to_short: [i16; 256],
    made_table: bool,
    stream_count: usize,
}

impl SourceWaveletCodec {
    /// Codec over a chunk allocator.
    pub fn new(allocate_chunk: Box<dyn FnMut() -> WaveletSoundChunk>) -> Self {
        Self {
            allocate_chunk,
            mulaw_to_short: [0; 256],
            made_table: false,
            stream_count: 0,
        }
    }

    /// Write one byte at the shared stream cursor (`NXPutc`).
    pub fn nx_putc(&mut self, stream: &mut [u8], output: i32) -> Result<(), AudioError> {
        integer(i64::from(output), -128, 255, "stream character")?;
        let offset = self.stream_count;
        self.stream_count += 1;
        if offset >= stream.len() {
            return Err(AudioError::NxPutcBounds);
        }
        stream[offset] = output as u8;
        Ok(())
    }

    fn make_table(&mut self) {
        if self.made_table {
            return;
        }
        for (index, slot) in self.mulaw_to_short.iter_mut().enumerate() {
            *slot = mu_law_decode(index as u8);
        }
        self.made_table = true;
    }

    /// Read the owned table, including its initial all-zero state.
    pub fn mu_law_sample(&self, value: u8) -> i16 {
        self.mulaw_to_short[usize::from(value)]
    }

    /// Compress packets with the wavelet transform.
    pub fn encode_wavelet(&mut self, sound: &mut WaveletSound, packets: &[i16]) -> Result<(), AudioError> {
        self.make_table();
        integer(sound.sound_length as i64, 0, 2_147_483_647, "sound length")?;
        let mut chunks: Vec<WaveletSoundChunk> = Vec::new();
        let mut offset = 0usize;
        let mut remaining = sound.sound_length;
        while remaining > 0 {
            let size = 4.max(remaining.min(WAVELET_CHUNK_BYTES));
            let mut chunk = (self.allocate_chunk)();
            let mut scratch = vec![0f32; size];
            for slot in scratch.iter_mut() {
                *slot = packets.get(offset).copied().ok_or(AudioError::WaveletRead(offset))? as f32;
                offset += 1;
            }
            wt1(&mut scratch, size, 1)?;
            for (index, byte) in chunk.data.iter_mut().enumerate().take(size) {
                let sample = f64::from(scratch[index]).clamp(-32768.0, 32767.0).trunc() as i32;
                *byte = mu_law_encode(sample)?;
            }
            chunk.size = size;
            remaining -= size;
            chunks.push(chunk);
        }
        link_chunks(sound, chunks)
    }

    /// Decompress one chunk, optionally into a destination.
    pub fn decode_wavelet(&self, chunk: &WaveletSoundChunk, destination: Option<&mut [i16]>) -> Result<(), AudioError> {
        self.decode_wavelet_data(chunk.size, &chunk.data, destination)
    }

    /// Decompress raw chunk bytes, optionally into a destination.
    pub fn decode_wavelet_data(&self, size: usize, data: &[u8], destination: Option<&mut [i16]>) -> Result<(), AudioError> {
        integer(size as i64, 0, WAVELET_CHUNK_BYTES as i64, "wavelet chunk size")?;
        let mut scratch = vec![0f32; size];
        for (index, slot) in scratch.iter_mut().enumerate() {
            let byte = data.get(index).copied().ok_or(AudioError::WaveletRead(index))?;
            *slot = self.mu_law_sample(byte) as f32;
        }
        wt1(&mut scratch, size, -1)?;
        let Some(destination) = destination else {
            return Ok(());
        };
        for (index, value) in scratch.iter().enumerate() {
            output_sample(destination, index, f64::from(*value))?;
        }
        Ok(())
    }

    /// Compress packets with mu-law and error feedback.
    pub fn encode_mu_law(&mut self, sound: &mut WaveletSound, packets: &[i16]) -> Result<(), AudioError> {
        self.make_table();
        integer(sound.sound_length as i64, 0, 2_147_483_647, "sound length")?;
        let mut chunks: Vec<WaveletSoundChunk> = Vec::new();
        let mut offset = 0usize;
        let mut remaining = sound.sound_length;
        let mut grade = 0i32;
        while remaining > 0 {
            let size = remaining.min(WAVELET_CHUNK_BYTES);
            let mut chunk = (self.allocate_chunk)();
            for byte in chunk.data.iter_mut().take(size) {
                let packet = packets.get(offset).copied().ok_or(AudioError::WaveletRead(offset))?;
                let sample = (i32::from(packet) + grade).clamp(-32768, 32767);
                let encoded = mu_law_encode(sample)?;
                *byte = encoded;
                grade = sample - i32::from(self.mu_law_sample(encoded));
                offset += 1;
            }
            chunk.size = size;
            remaining -= size;
            chunks.push(chunk);
        }
        link_chunks(sound, chunks)
    }

    /// Decompress one mu-law chunk.
    pub fn decode_mu_law(&self, chunk: &WaveletSoundChunk, destination: &mut [i16]) -> Result<(), AudioError> {
        integer(chunk.size as i64, 0, WAVELET_CHUNK_BYTES as i64, "mu-law chunk size")?;
        for index in 0..chunk.size {
            let byte = chunk.data.get(index).copied().ok_or(AudioError::WaveletRead(index))?;
            output_sample(destination, index, f64::from(self.mu_law_sample(byte)))?;
        }
        Ok(())
    }
}

fn link_chunks(sound: &mut WaveletSound, chunks: Vec<WaveletSoundChunk>) -> Result<(), AudioError> {
    if sound.sound_data.is_some() {
        return Err(AudioError::WaveletSoundData);
    }
    let mut next: Option<Box<WaveletSoundChunk>> = None;
    for mut chunk in chunks.into_iter().rev() {
        chunk.next = next;
        next = Some(Box::new(chunk));
    }
    sound.sound_data = next;
    Ok(())
}

fn output_sample(destination: &mut [i16], index: usize, sample: f64) -> Result<(), AudioError> {
    if index >= destination.len() {
        return Err(AudioError::WaveletDest);
    }
    let truncated = sample.trunc() as i32;
    integer(i64::from(truncated), -32768, 32767, "decoded wavelet short")?;
    destination[index] = truncated as i16;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mulaw_round_trips() {
        assert_eq!(mu_law_encode(0).unwrap(), 0x7f);
        assert_eq!(mu_law_decode(0x7f), -4);
        assert_eq!(mu_law_decode(0xff), 4);
        for sample in [-32768, -1000, -1, 0, 1, 1000, 32767] {
            let decoded = mu_law_decode(mu_law_encode(sample).unwrap());
            let tolerance = (sample.abs() / 16).max(16);
            assert!((i32::from(decoded) - sample).abs() <= tolerance, "{sample} -> {decoded}");
        }
        assert!(mu_law_encode(32768).is_err());
        assert!(mu_law_encode(-32769).is_err());
    }

    #[test]
    fn wavelet_round_trips_chunks() {
        let packets: Vec<i16> = (0..3000).map(|index| ((index * 13) % 2000 - 1000) as i16).collect();
        let mut sound = WaveletSound {
            sound_length: packets.len(),
            sound_data: None,
        };
        let mut codec = SourceWaveletCodec::new(Box::new(|| WaveletSoundChunk::new([0; WAVELET_CHUNK_BYTES])));
        codec.encode_wavelet(&mut sound, &packets).unwrap();
        assert!(sound.sound_data.as_ref().unwrap().next.is_some());
        let mut decoded = vec![0i16; packets.len()];
        let mut chunk = sound.sound_data.as_ref();
        let mut offset = 0;
        while let Some(current) = chunk {
            let size = current.size;
            codec.decode_wavelet(current, Some(&mut decoded[offset..offset + size])).unwrap();
            offset += size;
            chunk = current.next.as_ref();
        }
        let error: i64 = packets.iter().zip(decoded.iter()).map(|(want, got)| i64::from(want - got).abs()).sum::<i64>() / packets.len() as i64;
        assert!(error < 600, "mean error {error}");
        assert!(daub4(&mut [0f32; 3], 3, 1).is_ok());
        assert!(wt1(&mut [0f32; 4], 2, 1).is_err());
    }
}
