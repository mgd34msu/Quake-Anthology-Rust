//! Shared audio capability. Prepare device-rate PCM once at precache.
use qa_core::primitives::Pcm;
use std::num::NonZeroU32;

mod mixer;
pub use mixer::{Bank, Listener, MixCounts, Mixer};

#[derive(Debug, PartialEq, Eq)]
pub enum PrepareError {
    InvalidPcm,
    TooLarge,
}

pub fn prepare(source: Pcm, rate: NonZeroU32, eight_bit: bool) -> Result<Pcm, PrepareError> {
    let channels = source.channels as usize;
    let count = source.frames();
    if !source.samples.len().is_multiple_of(channels)
        || source.loop_start.is_some_and(|start| start >= count)
    {
        return Err(PrepareError::InvalidPcm);
    }
    if source.rate == rate && !eight_bit {
        return Ok(source);
    }
    let ratio = source.rate.get() as f32 / rate.get() as f32;
    let frames = (count as f32 / ratio).trunc() as usize;
    if frames > 256 * 1024 * 1024 / (channels * 2) {
        return Err(PrepareError::TooLarge);
    }
    let loop_start = source
        .loop_start
        .map(|start| (start as f32 / ratio).trunc() as usize);
    if loop_start.is_some_and(|start| start >= frames) {
        return Err(PrepareError::InvalidPcm);
    }
    let step = (ratio * 256.0) as u64;
    let mut samples = vec![0; frames * channels];
    for frame in 0..frames {
        // Widen the native fixed-point clock to prevent long-sample overflow.
        let index = ((frame as u64 * step) >> 8) as usize;
        if index < count {
            for channel in 0..channels {
                let value = source.samples[index * channels + channel];
                samples[frame * channels + channel] =
                    if eight_bit { (value >> 8) << 8 } else { value };
            }
        }
    }
    Ok(Pcm {
        rate,
        channels: source.channels,
        samples,
        loop_start,
    })
}
