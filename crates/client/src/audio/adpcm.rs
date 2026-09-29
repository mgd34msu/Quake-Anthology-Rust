//! Intel/DVI ADPCM encode and decode.
//!
//! Donor provenance: `src/audio/adpcm.ts` (`S_AdpcmEncode`,
//! `S_AdpcmDecode`, `S_AdpcmEncodeSound`, `S_AdpcmMemoryNeeded`,
//! from `snd_adpcm.c`).

use super::error::AudioError;

/// Bytes per ADPCM chunk.
pub const ADPCM_CHUNK_BYTES: usize = 2048;
/// Samples per ADPCM chunk.
pub const ADPCM_CHUNK_SAMPLES: usize = 4096;

const INDEX_TABLE: [i32; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];

const STEP_SIZE_TABLE: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88, 97, 107, 118,
    130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658, 724, 796, 876, 963, 1060,
    1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484,
    7132, 7845, 8630, 9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];

/// ADPCM predictor state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AdpcmState {
    /// Predicted sample.
    pub sample: i32,
    /// Step-table index.
    pub index: i32,
}

fn require_state(state: &AdpcmState) -> Result<(), AudioError> {
    if !(-32768..=32767).contains(&state.sample) {
        return Err(AudioError::BadAdpcmPredictor);
    }
    if !(0..=88).contains(&state.index) {
        return Err(AudioError::BadAdpcmIndex);
    }
    Ok(())
}

/// One `sndBuffer` ADPCM chunk. The allocator owns the storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdpcmChunk {
    /// Chunk header state.
    pub adpcm: AdpcmState,
    /// Compressed bytes.
    pub data: [u8; ADPCM_CHUNK_BYTES],
    /// Next chunk.
    pub next: Option<Box<AdpcmChunk>>,
}

impl AdpcmChunk {
    /// Chunk over caller storage.
    #[must_use]
    pub const fn new(data: [u8; ADPCM_CHUNK_BYTES]) -> Self {
        Self {
            adpcm: AdpcmState { sample: 0, index: 0 },
            data,
            next: None,
        }
    }
}

/// Sound with ADPCM chunk data.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AdpcmSound {
    /// First chunk.
    pub sound_data: Option<Box<AdpcmChunk>>,
}

/// Encode samples; each call starts at the high nibble (`S_AdpcmEncode`).
pub fn encode_adpcm(samples: &[i16], output: &mut [u8], state: &mut AdpcmState) -> Result<(), AudioError> {
    require_state(state)?;
    if output.len() < samples.len().div_ceil(2) {
        return Err(AudioError::AdpcmOutputShort);
    }
    let mut predicted = state.sample;
    let mut index = state.index;
    let mut step = STEP_SIZE_TABLE[index as usize];
    let mut output_buffer = 0u8;
    let mut high_nibble = true;
    let mut output_offset = 0usize;
    for &sample in samples {
        let mut difference = i32::from(sample) - predicted;
        let sign = if difference < 0 { 8 } else { 0 };
        if sign != 0 {
            difference = -difference;
        }
        let mut delta = 0;
        let mut predicted_difference = step >> 3;
        if difference >= step {
            delta = 4;
            difference -= step;
            predicted_difference += step;
        }
        step >>= 1;
        if difference >= step {
            delta |= 2;
            difference -= step;
            predicted_difference += step;
        }
        step >>= 1;
        if difference >= step {
            delta |= 1;
            predicted_difference += step;
        }
        predicted = if sign != 0 {
            predicted - predicted_difference
        } else {
            predicted + predicted_difference
        }
        .clamp(-32768, 32767);
        delta |= sign;
        index = (index + INDEX_TABLE[delta as usize]).clamp(0, 88);
        step = STEP_SIZE_TABLE[index as usize];
        if high_nibble {
            output_buffer = ((delta << 4) & 0xf0) as u8;
        } else {
            output[output_offset] = (delta & 0x0f) as u8 | output_buffer;
            output_offset += 1;
        }
        high_nibble = !high_nibble;
    }
    if !high_nibble {
        output[output_offset] = output_buffer;
    }
    state.sample = predicted;
    state.index = index;
    Ok(())
}

/// Decode samples (`S_AdpcmDecode`).
pub fn decode_adpcm(input: &[u8], output: &mut [i16], state: &mut AdpcmState) -> Result<(), AudioError> {
    require_state(state)?;
    if input.len() < output.len().div_ceil(2) {
        return Err(AudioError::AdpcmInputTruncated);
    }
    let mut predicted = state.sample;
    let mut index = state.index;
    let mut step = STEP_SIZE_TABLE[index as usize];
    let mut input_buffer = 0u8;
    for (offset, slot) in output.iter_mut().enumerate() {
        let high_nibble = offset & 1 == 0;
        if high_nibble {
            input_buffer = input[offset >> 1];
        }
        let code = i32::from(if high_nibble { (input_buffer >> 4) & 0xf } else { input_buffer & 0xf });
        index = (index + INDEX_TABLE[code as usize]).clamp(0, 88);
        let sign = code & 8;
        let delta = code & 7;
        let mut predicted_difference = step >> 3;
        if delta & 4 != 0 {
            predicted_difference += step;
        }
        if delta & 2 != 0 {
            predicted_difference += step >> 1;
        }
        if delta & 1 != 0 {
            predicted_difference += step >> 2;
        }
        predicted = if sign != 0 {
            predicted - predicted_difference
        } else {
            predicted + predicted_difference
        }
        .clamp(-32768, 32767);
        step = STEP_SIZE_TABLE[index as usize];
        *slot = predicted as i16;
    }
    state.sample = predicted;
    state.index = index;
    Ok(())
}

/// Decode a full chunk without advancing its saved header.
pub fn decode_adpcm_chunk(chunk: &AdpcmChunk, output: &mut [i16]) -> Result<(), AudioError> {
    if output.len() < ADPCM_CHUNK_SAMPLES {
        return Err(AudioError::AdpcmChunkOutput);
    }
    let mut state = chunk.adpcm;
    decode_adpcm(&chunk.data, &mut output[..ADPCM_CHUNK_SAMPLES], &mut state)
}

/// Encode resampled mono PCM into linked chunks (`S_AdpcmEncodeSound`).
pub fn encode_adpcm_sound(
    samples: &[i16],
    sound: &mut AdpcmSound,
    allocate_chunk: &mut dyn FnMut() -> AdpcmChunk,
) -> Result<(), AudioError> {
    let Some(&first) = samples.first() else {
        return Err(AudioError::AdpcmMissingInitial);
    };
    if sound.sound_data.is_some() {
        return Err(AudioError::AdpcmSoundDataSet);
    }
    let mut state = AdpcmState {
        sample: i32::from(first),
        index: 0,
    };
    let mut chunks = Vec::new();
    for window in samples.chunks(ADPCM_CHUNK_SAMPLES) {
        let mut chunk = allocate_chunk();
        chunk.next = None;
        chunk.adpcm = state;
        encode_adpcm(window, &mut chunk.data, &mut state)?;
        chunks.push(chunk);
    }
    let mut next: Option<Box<AdpcmChunk>> = None;
    for mut chunk in chunks.into_iter().rev() {
        chunk.next = next;
        next = Some(Box::new(chunk));
    }
    sound.sound_data = next;
    Ok(())
}

/// Compressed size estimate (`S_AdpcmMemoryNeeded`).
pub fn adpcm_memory_needed(sample_count: i32, input_rate: i32, output_rate: i32) -> Result<i32, AudioError> {
    for value in [sample_count, input_rate, output_rate] {
        if value < 0 {
            return Err(AudioError::AdpcmMemoryInputs);
        }
    }
    if input_rate == 0 || output_rate == 0 {
        return Err(AudioError::AdpcmMemoryRates);
    }
    let scale = input_rate as f32 / output_rate as f32;
    let scaled = (sample_count as f32 / scale).trunc();
    if f64::from(scaled) > 2_147_483_647.0 {
        return Err(AudioError::AdpcmMemoryScaled);
    }
    let scaled = scaled as i32;
    let sample_memory = scaled / 2;
    let block_count = scaled.div_ceil(ADPCM_CHUNK_SAMPLES as i32);
    Ok(sample_memory + block_count * 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_statefully() {
        let samples: Vec<i16> = (0..6000).map(|index| ((index * 37) % 2000 - 1000) as i16).collect();
        let mut state = AdpcmState::default();
        let mut packed = vec![0u8; samples.len().div_ceil(2)];
        encode_adpcm(&samples, &mut packed, &mut state).unwrap();
        let mut decoded = vec![0i16; samples.len()];
        let mut restore = AdpcmState::default();
        decode_adpcm(&packed, &mut decoded, &mut restore).unwrap();
        assert_eq!(restore, state);
        let error: i64 = samples.iter().zip(decoded.iter()).map(|(want, got)| i64::from(want - got).abs()).sum::<i64>() / samples.len() as i64;
        assert!(error < 300, "mean error {error}");
    }

    #[test]
    fn chunks_link_and_decode() {
        let samples = vec![1000i16; ADPCM_CHUNK_SAMPLES + 10];
        let mut sound = AdpcmSound::default();
        encode_adpcm_sound(&samples, &mut sound, &mut || AdpcmChunk::new([0; ADPCM_CHUNK_BYTES])).unwrap();
        let first = sound.sound_data.as_ref().unwrap();
        assert!(first.next.is_some());
        assert_eq!(first.adpcm.sample, 1000);
        let mut output = vec![0i16; ADPCM_CHUNK_SAMPLES];
        decode_adpcm_chunk(first, &mut output).unwrap();
        assert!(output.iter().all(|sample| (*sample - 1000).abs() < 120));
        assert_eq!(adpcm_memory_needed(4096, 22050, 11025).unwrap(), 1024 + 4);
        assert!(encode_adpcm(&samples, &mut [0u8; 4], &mut AdpcmState::default()).is_err());
    }
}
