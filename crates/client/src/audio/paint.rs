//! Source PCM painting: resampling, blasting, DMA transfer, codecs.
//!
//! Donor provenance: `src/audio/source-paint.ts` (`SourcePaintChunk`,
//! `SourceSoundResampler`, `resampleSoundRaw`,
//! `writeLinearBlastStereo16`, `byteSwapRawSamples`,
//! `transferPaintBuffer`, `SourceCompressedPainter`, from `snd_mix.c`,
//! `snd_dma.c`, `snd_mem.c`).

use super::adpcm::{decode_adpcm, AdpcmState};
use super::error::AudioError;
use super::wavelet::SourceWaveletCodec;

/// JavaScript `ToInt32` (`value | 0`).
#[must_use]
pub fn int32(value: f64) -> i32 {
    if !value.is_finite() {
        return 0;
    }
    let wrapped = value.trunc() % 4_294_967_296.0;
    let wrapped = if wrapped < 0.0 {
        wrapped + 4_294_967_296.0
    } else {
        wrapped
    };
    if wrapped >= 2_147_483_648.0 {
        (wrapped - 4_294_967_296.0) as i32
    } else {
        wrapped as i32
    }
}

/// Paint chunk: ADPCM header plus shared 2048-byte storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePaintChunk {
    /// ADPCM header state.
    pub adpcm: AdpcmState,
    /// Shared chunk storage.
    pub data: [u8; 2048],
    /// Next chunk.
    pub next: Option<Box<SourcePaintChunk>>,
    /// Written bytes.
    pub size: usize,
}

impl SourcePaintChunk {
    /// Chunk over caller storage.
    #[must_use]
    pub const fn new(data: [u8; 2048]) -> Self {
        Self {
            adpcm: AdpcmState { sample: 0, index: 0 },
            data,
            next: None,
            size: 0,
        }
    }
}

/// Sound with paint chunks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SourcePaintSound {
    /// First chunk.
    pub sound_data: Option<Box<SourcePaintChunk>>,
}

/// Paint channel volumes and Doppler.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourcePaintChannel {
    /// Left volume.
    pub leftvol: i32,
    /// Right volume.
    pub rightvol: i32,
    /// Doppler active.
    pub doppler: bool,
    /// Doppler scale.
    pub doppler_scale: f64,
    /// Previous Doppler scale.
    pub old_doppler_scale: f64,
}

/// `ResampleSfx`/`ResampleSfxRaw` sizing with the wrapped 8.8 cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceSoundResampler {
    /// Output frame count.
    pub count: i32,
    step: i32,
}

impl SourceSoundResampler {
    /// Resampler for a rate conversion.
    pub fn new(input_rate: f64, output_rate: f64, sample_count: f64) -> Result<Self, AudioError> {
        let scale = (input_rate as f32) / (output_rate as f32);
        let count = ((sample_count as f32) / scale).trunc();
        let step = (scale * 256.0).trunc();
        for value in [count, step] {
            if !value.is_finite() || f64::from(value) < -2_147_483_648.0 || f64::from(value) > 2_147_483_647.0 {
                return Err(AudioError::ResampleConversion);
            }
        }
        Ok(Self {
            count: count as i32,
            step: step as i32,
        })
    }

    /// Source index for an output frame.
    #[must_use]
    pub const fn source_index(&self, frame: i32) -> i32 {
        frame.wrapping_mul(self.step) >> 8
    }
}

/// Resample raw bytes into shorts (`ResampleSfxRaw`).
pub fn resample_sound_raw(
    output: &mut [i16],
    input_rate: f64,
    output_rate: f64,
    width: i32,
    samples: f64,
    data: &[u8],
) -> Result<i32, AudioError> {
    let resampler = SourceSoundResampler::new(input_rate, output_rate, samples)?;
    for frame in 0..resampler.count.max(0) {
        let source = resampler.source_index(frame);
        let sample = if width == 2 {
            let offset = source as i64 * 2;
            if offset < 0 || offset + 1 >= data.len() as i64 {
                return Err(AudioError::PaintAccess(offset));
            }
            let offset = offset as usize;
            i32::from(i16::from_le_bytes([data[offset], data[offset + 1]]))
        } else {
            let offset = source as i64;
            if offset < 0 || offset >= data.len() as i64 {
                return Err(AudioError::PaintAccess(offset));
            }
            (i32::from(data[offset as usize]) - 128) << 8
        };
        if frame as usize >= output.len() {
            return Err(AudioError::ResampleTruncated);
        }
        output[frame as usize] = sample as i16;
    }
    Ok(resampler.count)
}

fn clipped(value: i32) -> i16 {
    (value >> 8).clamp(-32768, 32767) as i16
}

/// Whether float gains can take the integer fast path exactly.
///
/// Finite integral gains below 2^37 keep `sample * gain` inside `i64` and
/// below 2^53, where the `f64` multiply is exact, so both paths agree bit for bit.
#[must_use]
pub fn integral_gains(left_gain: f64, right_gain: f64) -> bool {
    const BOUND: f64 = 137_438_953_472.0;
    left_gain.is_finite()
        && right_gain.is_finite()
        && left_gain.trunc() == left_gain
        && right_gain.trunc() == right_gain
        && left_gain.abs() < BOUND
        && right_gain.abs() < BOUND
}

/// Paint one resampled span into integer paint with integral gains.
///
/// Each contribution is `(sample * gain) >> 8`, which equals
/// `floor(sample * gain / 256)` for integral inputs, so this matches the
/// float paint path bit for bit. Range-checked once per span.
pub fn paint_span_i32(
    paint: &mut [i32],
    out_start: usize,
    samples: &[i16],
    left_gain: i64,
    right_gain: i64,
) -> Result<(), AudioError> {
    let len = paint.len();
    let end = out_start
        .checked_add(samples.len())
        .ok_or_else(|| AudioError::BadPaintIndex {
            index: out_start.to_string(),
            length: len.to_string(),
        })?;
    let paint = paint
        .get_mut(out_start * 2..end * 2)
        .ok_or_else(|| AudioError::BadPaintIndex {
            index: (end * 2).to_string(),
            length: len.to_string(),
        })?;
    let (slots, _) = paint.as_chunks_mut::<2>();
    for (slot, sample) in slots.iter_mut().zip(samples.iter()) {
        slot[0] = slot[0].wrapping_add((((i64::from(*sample)) * left_gain) >> 8) as i32);
        slot[1] = slot[1].wrapping_add((((i64::from(*sample)) * right_gain) >> 8) as i32);
    }
    Ok(())
}

/// Paint one resampled span with fractional gains (ambient fades).
///
/// Contributions are `floor(sample * gain / 256)` exactly like the float
/// path, accumulated into integer paint. Range-checked once per span.
pub fn paint_span_f64(
    paint: &mut [i32],
    out_start: usize,
    samples: &[i16],
    left_gain: f64,
    right_gain: f64,
) -> Result<(), AudioError> {
    let len = paint.len();
    let end = out_start
        .checked_add(samples.len())
        .ok_or_else(|| AudioError::BadPaintIndex {
            index: out_start.to_string(),
            length: len.to_string(),
        })?;
    let paint = paint
        .get_mut(out_start * 2..end * 2)
        .ok_or_else(|| AudioError::BadPaintIndex {
            index: (end * 2).to_string(),
            length: len.to_string(),
        })?;
    let (slots, _) = paint.as_chunks_mut::<2>();
    for (slot, sample) in slots.iter_mut().zip(samples.iter()) {
        slot[0] = slot[0].wrapping_add((f64::from(*sample) * left_gain / 256.0).floor() as i32);
        slot[1] = slot[1].wrapping_add((f64::from(*sample) * right_gain / 256.0).floor() as i32);
    }
    Ok(())
}

/// Paint one bank-memory sample with float gains.
pub fn paint_sample_f64(
    paint: &mut [i32],
    out_frame: usize,
    sample: i32,
    left_gain: f64,
    right_gain: f64,
) -> Result<(), AudioError> {
    let len = paint.len();
    let base = out_frame.checked_mul(2).ok_or_else(|| AudioError::BadPaintIndex {
        index: out_frame.to_string(),
        length: len.to_string(),
    })?;
    let slot = paint.get_mut(base..base + 2).ok_or_else(|| AudioError::BadPaintIndex {
        index: (base + 1).to_string(),
        length: len.to_string(),
    })?;
    slot[0] = slot[0].wrapping_add((f64::from(sample) * left_gain / 256.0).floor() as i32);
    slot[1] = slot[1].wrapping_add((f64::from(sample) * right_gain / 256.0).floor() as i32);
    Ok(())
}

/// Blast an integer paint buffer to interleaved stereo.
pub fn write_linear_blast_stereo16(paint: &[i32], output: &mut [i16], count: usize) -> Result<(), AudioError> {
    if !count.is_multiple_of(2) {
        return Err(AudioError::BlastCount);
    }
    for index in (0..count).step_by(2) {
        if index + 1 >= output.len() {
            return Err(AudioError::BlastOutput);
        }
        output[index] = clipped(paint.get(index).copied().ok_or(AudioError::PaintAccess(index as i64))?);
        output[index + 1] = clipped(
            paint
                .get(index + 1)
                .copied()
                .ok_or(AudioError::PaintAccess(index as i64 + 1))?,
        );
    }
    Ok(())
}

/// Blast a float paint buffer to interleaved stereo.
pub fn write_linear_blast_stereo16_float(paint: &[f64], output: &mut [i16], count: usize) -> Result<(), AudioError> {
    if !count.is_multiple_of(2) {
        return Err(AudioError::BlastCount);
    }
    for index in (0..count).step_by(2) {
        if index + 1 >= output.len() {
            return Err(AudioError::BlastOutput);
        }
        for offset in [0, 1] {
            let sample = paint
                .get(index + offset)
                .copied()
                .ok_or(AudioError::PaintAccess(index as i64 + offset as i64))?;
            output[index + offset] = (sample / 256.0).floor().clamp(-32768.0, 32767.0) as i16;
        }
    }
    Ok(())
}

/// Circular DMA buffer.
pub enum SourceDmaBuffer<'a> {
    /// 16-bit samples.
    S16 {
        /// Channel count.
        channels: u8,
        /// Ring samples.
        samples: &'a mut [i16],
    },
    /// 8-bit samples.
    S8 {
        /// Channel count.
        channels: u8,
        /// Ring samples.
        samples: &'a mut [u8],
    },
}

impl SourceDmaBuffer<'_> {
    fn channels(&self) -> usize {
        match self {
            SourceDmaBuffer::S16 { channels, .. } | SourceDmaBuffer::S8 { channels, .. } => usize::from(*channels),
        }
    }

    fn capacity(&self) -> usize {
        match self {
            SourceDmaBuffer::S16 { samples, .. } => samples.len(),
            SourceDmaBuffer::S8 { samples, .. } => samples.len(),
        }
    }
}

/// Swap raw sample bytes on big-endian hosts (`S_ByteSwapRawSamples`).
pub fn byte_swap_raw_samples(
    samples: i32,
    width: i32,
    channels: i32,
    data: &mut [u8],
    little_endian: bool,
) -> Result<(), AudioError> {
    if width != 2 || little_endian {
        return Ok(());
    }
    let samples = if channels == 2 {
        samples.wrapping_shl(1)
    } else {
        samples
    };
    for index in 0..samples.max(0) as usize {
        let offset = index * 2;
        let first = data
            .get(offset)
            .copied()
            .ok_or(AudioError::PaintAccess(offset as i64))?;
        let second = data
            .get(offset + 1)
            .copied()
            .ok_or(AudioError::PaintAccess(offset as i64 + 1))?;
        data[offset] = second;
        data[offset + 1] = first;
    }
    Ok(())
}

/// Transfer paint blocks into the DMA ring (`S_TransferPaintBuffer`).
pub fn transfer_paint_buffer(
    paint: &mut [i32],
    dma: &mut SourceDmaBuffer,
    painted_time: i64,
    end_time: i64,
    test_sound: bool,
) -> Result<(), AudioError> {
    let frames = end_time - painted_time;
    let capacity = dma.capacity();
    if frames < 0 || capacity < dma.channels() || !capacity.is_power_of_two() {
        return Err(AudioError::DmaRange);
    }
    if test_sound {
        for index in 0..frames as usize {
            if index * 2 + 1 >= paint.len() {
                return Err(AudioError::PaintTruncated);
            }
            let sample = ((painted_time + index as i64) as f64 * 0.1).sin() * 20000.0 * 256.0;
            paint[index * 2] = sample.trunc() as i32;
            paint[index * 2 + 1] = sample.trunc() as i32;
        }
    }
    let count = frames as usize * dma.channels();
    let step = 3 - dma.channels();
    let mask = capacity - 1;
    let mut destination = ((painted_time * dma.channels() as i64) & mask as i64) as usize;
    for index in 0..count {
        let sample = clipped(
            paint
                .get(index * step)
                .copied()
                .ok_or(AudioError::PaintAccess((index * step) as i64))?,
        );
        match dma {
            SourceDmaBuffer::S16 { samples, .. } => samples[destination] = sample,
            SourceDmaBuffer::S8 { samples, .. } => samples[destination] = ((sample as i32 >> 8) + 128) as u8,
        }
        destination = (destination + 1) & mask;
    }
    Ok(())
}

/// Compressed painter with the shared decode scratch.
pub struct SourceCompressedPainter {
    scratch: [i16; 4096],
    scratch_sound: Option<*const SourcePaintSound>,
    scratch_index: usize,
}

impl SourceCompressedPainter {
    /// Fresh painter.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            scratch: [0; 4096],
            scratch_sound: None,
            scratch_index: 0,
        }
    }

    fn add(
        paint: &mut [i32],
        index: usize,
        sample: i32,
        channel: &SourcePaintChannel,
        volume: f64,
    ) -> Result<(), AudioError> {
        let gain = int32(volume);
        let left = sample.wrapping_mul(channel.leftvol.wrapping_mul(gain)) >> 8;
        let right = sample.wrapping_mul(channel.rightvol.wrapping_mul(gain)) >> 8;
        let slot = paint
            .get_mut(index * 2)
            .ok_or(AudioError::PaintAccess(index as i64 * 2))?;
        *slot = slot.wrapping_add(left);
        let slot = paint
            .get_mut(index * 2 + 1)
            .ok_or(AudioError::PaintAccess(index as i64 * 2 + 1))?;
        *slot = slot.wrapping_add(right);
        Ok(())
    }

    /// Paint ADPCM samples.
    #[allow(clippy::too_many_arguments)]
    pub fn paint_adpcm(
        &mut self,
        sound: &SourcePaintSound,
        channel: &SourcePaintChannel,
        paint: &mut [i32],
        count: usize,
        mut sample_offset: i64,
        buffer_offset: usize,
        volume: f64,
    ) -> Result<(), AudioError> {
        let mut current = sound.sound_data.as_ref().ok_or(AudioError::NullPaintChunk)?;
        let mut index = 0usize;
        if channel.doppler {
            sample_offset = (f64::from(sample_offset as f32) * channel.old_doppler_scale) as f32 as i64;
        }
        while sample_offset >= 4096 {
            current = current.next.as_ref().ok_or(AudioError::NullPaintChunk)?;
            sample_offset -= 4096;
            index += 1;
        }
        if index != self.scratch_index || self.scratch_sound != Some(sound as *const SourcePaintSound) {
            decode_chunk(current, &mut self.scratch)?;
            self.scratch_index = index;
            self.scratch_sound = Some(sound as *const SourcePaintSound);
        }
        for index in 0..count {
            let sample = self
                .scratch
                .get(sample_offset as usize)
                .copied()
                .ok_or(AudioError::PaintAccess(sample_offset))?;
            Self::add(paint, buffer_offset + index, i32::from(sample), channel, volume)?;
            sample_offset += 1;
            if sample_offset == 4096 {
                current = current.next.as_ref().ok_or(AudioError::NullPaintChunk)?;
                decode_chunk(current, &mut self.scratch)?;
                sample_offset = 0;
                self.scratch_index += 1;
            }
        }
        Ok(())
    }

    /// Paint wavelet samples (ADPCM at the first boundary by design).
    #[allow(clippy::too_many_arguments)]
    pub fn paint_wavelet(
        &mut self,
        codec: &SourceWaveletCodec,
        sound: &SourcePaintSound,
        channel: &SourcePaintChannel,
        paint: &mut [i32],
        count: usize,
        mut sample_offset: i64,
        buffer_offset: usize,
        volume: f64,
    ) -> Result<(), AudioError> {
        let mut current = sound.sound_data.as_ref().ok_or(AudioError::NullPaintChunk)?;
        let mut index = 0usize;
        while sample_offset >= 2048 {
            current = current.next.as_ref().ok_or(AudioError::NullPaintChunk)?;
            sample_offset -= 2048;
            index += 1;
        }
        if index != self.scratch_index || self.scratch_sound != Some(sound as *const SourcePaintSound) {
            decode_chunk(current, &mut self.scratch)?;
            self.scratch_index = index;
            self.scratch_sound = Some(sound as *const SourcePaintSound);
        }
        for index in 0..count {
            let sample = self
                .scratch
                .get(sample_offset as usize)
                .copied()
                .ok_or(AudioError::PaintAccess(sample_offset))?;
            Self::add(paint, buffer_offset + index, i32::from(sample), channel, volume)?;
            sample_offset += 1;
            if sample_offset == 2048 {
                current = current.next.as_ref().ok_or(AudioError::NullPaintChunk)?;
                codec.decode_wavelet_data(current.size, &current.data, Some(&mut self.scratch[..]))?;
                self.scratch_index += 1;
                sample_offset = 0;
            }
        }
        Ok(())
    }

    /// Paint mu-law samples.
    #[allow(clippy::too_many_arguments)]
    pub fn paint_mu_law(
        &mut self,
        codec: &SourceWaveletCodec,
        sound: &SourcePaintSound,
        channel: &SourcePaintChannel,
        paint: &mut [i32],
        count: usize,
        mut sample_offset: i64,
        buffer_offset: usize,
        volume: f64,
    ) -> Result<(), AudioError> {
        let mut current = sound.sound_data.as_deref().ok_or(AudioError::NullPaintChunk)?;
        while sample_offset >= 2048 {
            current = current
                .next
                .as_deref()
                .or(sound.sound_data.as_deref())
                .ok_or(AudioError::NullPaintChunk)?;
            sample_offset -= 2048;
        }
        if !channel.doppler {
            for index in 0..count {
                let byte = current
                    .data
                    .get(sample_offset as usize)
                    .copied()
                    .ok_or(AudioError::PaintAccess(sample_offset))?;
                sample_offset += 1;
                Self::add(
                    paint,
                    buffer_offset + index,
                    i32::from(codec.mu_law_sample(byte)),
                    channel,
                    volume,
                )?;
                if sample_offset == 2048 {
                    current = current.next.as_deref().ok_or(AudioError::NullPaintChunk)?;
                    sample_offset = 0;
                }
            }
        } else {
            let mut offset = sample_offset as f32;
            for index in 0..count {
                let at = offset.trunc();
                if at < 0.0 {
                    return Err(AudioError::PaintAccess(at as i64));
                }
                let byte = current
                    .data
                    .get(at as usize)
                    .copied()
                    .ok_or(AudioError::PaintAccess(at as i64))?;
                offset = (f64::from(offset) + channel.doppler_scale) as f32;
                Self::add(
                    paint,
                    buffer_offset + index,
                    i32::from(codec.mu_law_sample(byte)),
                    channel,
                    volume,
                )?;
                if offset >= 2048.0 {
                    current = current
                        .next
                        .as_deref()
                        .or(sound.sound_data.as_deref())
                        .ok_or(AudioError::NullPaintChunk)?;
                    offset = 0.0;
                }
            }
        }
        Ok(())
    }
}

impl Default for SourceCompressedPainter {
    fn default() -> Self {
        Self::new()
    }
}

fn decode_chunk(chunk: &SourcePaintChunk, scratch: &mut [i16; 4096]) -> Result<(), AudioError> {
    let mut state = chunk.adpcm;
    decode_adpcm(&chunk.data, &mut scratch[..], &mut state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampler_sizes_and_cursors() {
        let resampler = SourceSoundResampler::new(22050.0, 11025.0, 100.0).unwrap();
        assert_eq!(resampler.count, 50);
        assert_eq!(resampler.source_index(1), 2);
        assert!(SourceSoundResampler::new(1.0, 0.0, 10.0).is_err());
        let mut output = [0i16; 4];
        let data = [0u8, 128, 255, 64];
        assert_eq!(
            resample_sound_raw(&mut output, 11025.0, 11025.0, 1, 4.0, &data).unwrap(),
            4
        );
        assert_eq!(output, [(-128 << 8) as i16, 0, (127 << 8) as i16, (-64 << 8) as i16]);
    }

    #[test]
    fn span_paint_matches_float_path() {
        let samples: Vec<i16> = vec![0, 1, -1, 1000, -2000, 32767, -32768, 12345];
        // Integral gains: integer spans equal per-sample float floor.
        for (left, right) in [(127.0f64, 127.0f64), (25908.0, 13000.0), (0.0, 255.0)] {
            assert!(integral_gains(left, right));
            let mut paint = [100i32, -100, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
            paint_span_i32(&mut paint, 0, &samples, left as i64, right as i64).unwrap();
            for (index, sample) in samples.iter().enumerate() {
                let left_want = (f64::from(*sample) * left / 256.0).floor() as i32 + [100, 0, 0, 0, 0, 0, 0, 0][index];
                let right_want =
                    (f64::from(*sample) * right / 256.0).floor() as i32 + [-100, 0, 0, 0, 0, 0, 0, 0][index];
                assert_eq!(paint[index * 2], left_want, "gain {left}/{right} sample {sample}");
                assert_eq!(paint[index * 2 + 1], right_want);
            }
        }
        // Fractional gains take the float span with the same floor.
        assert!(!integral_gains(12.5, 127.0));
        let mut paint = [0i32; 6];
        paint_span_f64(&mut paint, 1, &samples[..2], 12.5, -3.25).unwrap();
        assert_eq!(paint[0], 0);
        assert_eq!(paint[1], 0);
        for (index, sample) in samples[..2].iter().enumerate() {
            assert_eq!(paint[2 + index * 2], (f64::from(*sample) * 12.5 / 256.0).floor() as i32);
            assert_eq!(
                paint[2 + index * 2 + 1],
                (f64::from(*sample) * -3.25 / 256.0).floor() as i32
            );
        }
        paint_sample_f64(&mut paint, 0, 1000, 25908.0, 25908.0).unwrap();
        assert_eq!(paint[0], (1000.0f64 * 25908.0 / 256.0).floor() as i32);
        assert!(paint_span_i32(&mut paint, 3, &samples, 1, 1).is_err());
        assert!(paint_sample_f64(&mut paint, 3, 0, 1.0, 1.0).is_err());
    }

    #[test]
    fn blasts_and_transfers() {
        let mut output = [0i16; 4];
        write_linear_blast_stereo16(&[256, 512, -256, -512], &mut output, 4).unwrap();
        assert_eq!(output, [1, 2, -1, -2]);
        write_linear_blast_stereo16_float(&[256.0, 511.0, -1.0, -257.0], &mut output, 4).unwrap();
        assert_eq!(output, [1, 1, -1, -2]);
        assert!(write_linear_blast_stereo16(&[0], &mut output, 3).is_err());
        let mut paint = [256i32, 512, 768, 1024];
        let mut ring = [0i16; 4];
        transfer_paint_buffer(
            &mut paint,
            &mut SourceDmaBuffer::S16 {
                channels: 2,
                samples: &mut ring,
            },
            0,
            2,
            false,
        )
        .unwrap();
        assert_eq!(ring, [1, 2, 3, 4]);
        let mut bytes = [0u8, 1, 2, 3];
        byte_swap_raw_samples(2, 2, 1, &mut bytes, false).unwrap();
        assert_eq!(bytes, [1, 0, 3, 2]);
    }
}
