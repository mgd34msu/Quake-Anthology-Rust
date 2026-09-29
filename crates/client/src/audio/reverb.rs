//! Freeverb reverb and the underwater high-shelf filter.
//!
//! Donor provenance: `src/audio/reverb.ts` (`StereoReverb`,
//! `UnderwaterFilter`, adapted from `snd_reverb_dsp.ts`).

use std::f64::consts::PI;

use super::error::AudioError;
use super::reverb_presets::EfxReverbParams;

struct Comb {
    buffer: Vec<f64>,
    index: usize,
    store: f64,
    feedback: f64,
    damping: f64,
}

impl Comb {
    fn new(length: usize) -> Self {
        Self {
            buffer: vec![0.0; length.max(1)],
            index: 0,
            store: 0.0,
            feedback: 0.0,
            damping: 0.0,
        }
    }

    fn process(&mut self, input: f64) -> f64 {
        let output = self.buffer[self.index];
        self.store = output * (1.0 - self.damping) + self.store * self.damping;
        self.buffer[self.index] = input + self.store * self.feedback;
        self.index = (self.index + 1) % self.buffer.len();
        output
    }

    fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.store = 0.0;
        self.index = 0;
    }
}

struct Allpass {
    buffer: Vec<f64>,
    index: usize,
    feedback: f64,
}

impl Allpass {
    fn new(length: usize) -> Self {
        Self {
            buffer: vec![0.0; length.max(1)],
            index: 0,
            feedback: 0.5,
        }
    }

    fn process(&mut self, input: f64) -> f64 {
        let old = self.buffer[self.index];
        self.buffer[self.index] = input + old * self.feedback;
        self.index = (self.index + 1) % self.buffer.len();
        old - input
    }

    fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.index = 0;
    }
}

struct Network {
    combs: Vec<Comb>,
    allpasses: Vec<Allpass>,
}

impl Network {
    fn new(rate: u32, spread: i32) -> Self {
        Self {
            combs: [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617]
                .iter()
                .map(|tuning| Comb::new((((tuning + spread) as f64 * f64::from(rate) / 44100.0).round().max(1.0)) as usize))
                .collect(),
            allpasses: [556, 441, 341, 225]
                .iter()
                .map(|tuning| Allpass::new((((tuning + spread) as f64 * f64::from(rate) / 44100.0).round().max(1.0)) as usize))
                .collect(),
        }
    }

    fn process(&mut self, input: f64) -> f64 {
        let mut output = 0.0;
        for comb in &mut self.combs {
            output += comb.process(input);
        }
        for filter in &mut self.allpasses {
            output = filter.process(output);
        }
        output
    }

    fn configure(&mut self, params: &EfxReverbParams, rate: u32) {
        let damping = (0.0f64.max(1.0 - params.decay_hf_ratio) + if params.decay_hf_limit { 0.05 } else { 0.0 }).min(0.99);
        for comb in &mut self.combs {
            comb.feedback = 10f64.powf(-3.0 * comb.buffer.len() as f64 / (params.decay_time.max(0.001) * f64::from(rate))).min(0.98);
            comb.damping = damping;
        }
        let diffusion = (params.diffusion * 0.7 + params.density * 0.3).clamp(0.0, 1.0);
        for filter in &mut self.allpasses {
            filter.feedback = 0.6 * diffusion;
        }
    }

    fn reset(&mut self) {
        for comb in &mut self.combs {
            comb.reset();
        }
        for filter in &mut self.allpasses {
            filter.reset();
        }
    }
}

/// Stereo Freeverb owned by one listener.
pub struct StereoReverb {
    left: Network,
    right: Network,
    delay_left: Vec<f64>,
    delay_right: Vec<f64>,
    position: usize,
    shelf_left: f64,
    shelf_right: f64,
    sample_rate: u32,
}

impl StereoReverb {
    /// Reverb at a sample rate.
    pub fn new(sample_rate: u32) -> Result<Self, AudioError> {
        if !(8000..=192000).contains(&sample_rate) {
            return Err(AudioError::BadReverbRate);
        }
        let delay = (f64::from(sample_rate) * 0.3).ceil() as usize + 1;
        Ok(Self {
            left: Network::new(sample_rate, 0),
            right: Network::new(sample_rate, 23),
            delay_left: vec![0.0; delay],
            delay_right: vec![0.0; delay],
            position: 0,
            shelf_left: 0.0,
            shelf_right: 0.0,
            sample_rate,
        })
    }

    /// Sample rate in Hz.
    #[must_use]
    pub const fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Process interleaved stereo frames in place.
    pub fn process(&mut self, samples: &mut [f64], params: &EfxReverbParams) -> Result<(), AudioError> {
        if samples.len() % 2 != 0 {
            return Err(AudioError::ReverbStereo);
        }
        self.left.configure(params, self.sample_rate);
        self.right.configure(params, self.sample_rate);
        let delay = (params.late_reverb_delay * f64::from(self.sample_rate)).round().clamp(0.0, self.delay_left.len() as f64 - 1.0) as usize;
        let shelf = 1.0 - (-2.0 * PI * params.hf_reference.clamp(20.0, f64::from(self.sample_rate) * 0.45) / f64::from(self.sample_rate)).exp();
        let wet_gain = (params.gain * (params.late_reverb_gain + 0.25 * params.reflections_gain)).clamp(0.0, 4.0) * 0.12;
        for pair in samples.chunks_mut(2) {
            let (dry_left, dry_right) = (pair[0], pair[1]);
            self.delay_left[self.position] = dry_left;
            self.delay_right[self.position] = dry_right;
            let index = (self.position + self.delay_left.len() - delay) % self.delay_left.len();
            let mut wet_left = self.left.process(self.delay_left[index] * 0.015);
            let mut wet_right = self.right.process(self.delay_right[index] * 0.015);
            self.position = (self.position + 1) % self.delay_left.len();
            self.shelf_left += shelf * (wet_left - self.shelf_left);
            self.shelf_right += shelf * (wet_right - self.shelf_right);
            wet_left = self.shelf_left + params.gain_hf * (wet_left - self.shelf_left);
            wet_right = self.shelf_right + params.gain_hf * (wet_right - self.shelf_right);
            pair[0] = dry_left + wet_left * wet_gain;
            pair[1] = dry_right + wet_right * wet_gain;
        }
        Ok(())
    }

    /// Clear delay lines and shelves.
    pub fn reset(&mut self) {
        self.left.reset();
        self.right.reset();
        self.delay_left.fill(0.0);
        self.delay_right.fill(0.0);
        self.position = 0;
        self.shelf_left = 0.0;
        self.shelf_right = 0.0;
    }
}

/// Underwater high-shelf biquad pair.
#[derive(Debug, Clone, Copy)]
struct BiquadState {
    z1: f64,
    z2: f64,
}

/// Underwater high-shelf filter, applied per seat.
pub struct UnderwaterFilter {
    left: BiquadState,
    right: BiquadState,
    sample_rate: u32,
}

impl UnderwaterFilter {
    /// Filter at a sample rate.
    #[must_use]
    pub const fn new(sample_rate: u32) -> Self {
        Self {
            left: BiquadState { z1: 0.0, z2: 0.0 },
            right: BiquadState { z1: 0.0, z2: 0.0 },
            sample_rate,
        }
    }

    /// Sample rate in Hz.
    #[must_use]
    pub const fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Process interleaved samples in place.
    pub fn process(&mut self, samples: &mut [f64], high_frequency_gain: f64) {
        let gain = high_frequency_gain.clamp(0.001, 1.0);
        let w = 2.0 * PI * (f64::from(self.sample_rate) * 0.45).min(5000.0) / f64::from(self.sample_rate);
        let c = w.cos();
        let alpha = w.sin() / 2.0 * std::f64::consts::SQRT_2;
        let k = 2.0 * gain.sqrt() * alpha;
        let a0 = gain + 1.0 - (gain - 1.0) * c + k;
        let b0 = gain * (gain + 1.0 + (gain - 1.0) * c + k) / a0;
        let b1 = -2.0 * gain * (gain - 1.0 + (gain + 1.0) * c) / a0;
        let b2 = gain * (gain + 1.0 + (gain - 1.0) * c - k) / a0;
        let a1 = 2.0 * (gain - 1.0 - (gain + 1.0) * c) / a0;
        let a2 = (gain + 1.0 - (gain - 1.0) * c - k) / a0;
        for (index, sample) in samples.iter_mut().enumerate() {
            let state = if index % 2 == 0 { &mut self.left } else { &mut self.right };
            let input = *sample;
            let output = input * b0 + state.z1;
            state.z1 = input * b1 - output * a1 + state.z2;
            state.z2 = input * b2 - output * a2;
            *sample = output;
        }
    }

    /// Clear filter state.
    pub fn reset(&mut self) {
        self.left = BiquadState { z1: 0.0, z2: 0.0 };
        self.right = BiquadState { z1: 0.0, z2: 0.0 };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::reverb_presets::REVERB_PRESETS;

    #[test]
    fn reverb_tails_an_impulse() {
        let mut reverb = StereoReverb::new(44100).unwrap();
        let mut samples = vec![0.0; 44100 * 2];
        samples[0] = 10000.0;
        samples[1] = 10000.0;
        reverb.process(&mut samples, &REVERB_PRESETS[0]).unwrap();
        assert_eq!(samples[0], 10000.0);
        let tail: f64 = samples[2000..].iter().map(|sample| sample.abs()).sum();
        assert!(tail > 1.0, "tail {tail}");
        assert!(StereoReverb::new(4000).is_err());
        assert!(reverb.process(&mut [0.0; 3], &REVERB_PRESETS[0]).is_err());
        reverb.reset();
    }

    #[test]
    fn underwater_attenuates_highs() {
        let mut filter = UnderwaterFilter::new(44100);
        let mut samples = vec![0.0; 1024];
        for (index, sample) in samples.iter_mut().enumerate() {
            // Alternate each frame (both channels) at Nyquist.
            *sample = if index / 2 % 2 == 0 { 1000.0 } else { -1000.0 };
        }
        filter.process(&mut samples, 0.25);
        // Skip the start-up transient; steady-state highs are cut.
        let peak: f64 = samples[128..].iter().map(|sample| sample.abs()).fold(0.0, f64::max);
        assert!(peak < 1000.0, "peak {peak}");
    }
}
