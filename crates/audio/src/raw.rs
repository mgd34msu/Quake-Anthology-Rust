//! Source-rate streaming PCM enters the same paint accumulator as effects.
use qa_core::primitives::{PcmChannels, PcmFormat};
use std::num::NonZeroU32;

pub const RAW_FRAMES: usize = 16_384;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RawError {
    Alignment,
    Volume,
    Capacity,
}

pub(crate) struct Raw {
    samples: Box<[[i32; 2]]>,
    end: u64,
    paused: bool,
    suspended: u64,
}
impl Raw {
    pub fn load() -> Self {
        Self {
            samples: vec![[0; 2]; RAW_FRAMES].into_boxed_slice(),
            end: 0,
            paused: false,
            suspended: 0,
        }
    }
    pub fn clear(&mut self, painted: u64) {
        self.end = painted;
        self.suspended = 0;
    }
    pub fn pause(&mut self, paused: bool) {
        self.paused = paused;
    }
    fn playback_frame(&self, painted: u64) -> u64 {
        painted.saturating_sub(self.suspended)
    }
    pub fn remaining(&self, painted: u64) -> usize {
        RAW_FRAMES.saturating_sub(self.end.saturating_sub(self.playback_frame(painted)) as usize)
    }
    pub fn sample(&mut self, time: u64) -> [i32; 2] {
        if self.paused {
            self.suspended += 1;
            return [0; 2];
        }
        let time = self.playback_frame(time);
        if time < self.end {
            self.samples[time as usize & (RAW_FRAMES - 1)]
        } else {
            [0; 2]
        }
    }
    pub fn queue(
        &mut self,
        painted: u64,
        output_rate: NonZeroU32,
        format: PcmFormat,
        bytes: &[u8],
        volume: f32,
        signed_stereo_bytes: bool,
    ) -> Result<usize, RawError> {
        if !bytes.len().is_multiple_of(format.frame_bytes()) {
            return Err(RawError::Alignment);
        }
        if !volume.is_finite() || volume < 0. {
            return Err(RawError::Volume);
        }
        let source_frames = bytes.len() / format.frame_bytes();
        let scale = format.rate.get() as f32 / output_rate.get() as f32;
        let mut frames = (source_frames as f64 / f64::from(scale)).ceil() as usize;
        while frames > 0 && ((frames - 1) as f32 * scale) as usize >= source_frames {
            frames -= 1;
        }
        while ((frames as f32 * scale) as usize) < source_frames {
            frames = frames.checked_add(1).ok_or(RawError::Capacity)?;
        }
        if frames > self.remaining(painted) {
            return Err(RawError::Capacity);
        }
        let painted = self.playback_frame(painted);
        // Native S_RawSamples restarts i*scale for every submitted source block.
        // Retained output time, not effect-channel fixed-point resampling,
        // determines where that block enters the shared paint buffer.
        let gain = (256. * volume) as i32;
        let width = format.width.bytes();
        let stereo = format.channels == PcmChannels::Stereo;
        self.end = self.end.max(painted);
        for index in 0..frames {
            let source = (index as f32 * scale) as usize * format.frame_bytes();
            let left = format.width.sample(
                &bytes[source..source + width],
                stereo && signed_stereo_bytes,
            );
            let right = if stereo {
                format.width.sample(
                    &bytes[source + width..source + width * 2],
                    signed_stereo_bytes,
                )
            } else {
                left
            };
            self.samples[self.end as usize & (RAW_FRAMES - 1)] = [
                i32::from(left).wrapping_mul(gain),
                i32::from(right).wrapping_mul(gain),
            ];
            self.end += 1;
        }
        Ok(frames)
    }
}
