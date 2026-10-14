//! Native chunk averaging and a bounded periodic extension for wide rates.
use qa_core::primitives::{LoopDoppler, Pcm};

const CHUNK: usize = 1024;

/// Cold range sums avoid scanning an unbounded number of cycles per sample.
/// Zero-filled chunk tails match the TypeScript defined PCM representation.
pub(crate) struct Prepared {
    prefix: Box<[i64]>,
    channels: usize,
    frames: usize,
    period: usize,
}
impl Prepared {
    pub(crate) fn load(pcm: &Pcm) -> Self {
        let frames = pcm.frames();
        let channels = pcm.channels as usize;
        let mut prefix = vec![0i64; pcm.samples.len() + channels];
        for frame in 0..frames {
            for side in 0..channels {
                prefix[(frame + 1) * channels + side] = prefix[frame * channels + side]
                    + i64::from(pcm.samples[frame * channels + side]);
            }
        }
        Self {
            prefix: prefix.into_boxed_slice(),
            channels,
            frames,
            period: frames.div_ceil(CHUNK) * CHUNK,
        }
    }
    fn range(&self, side: usize, first: usize, last: usize) -> i64 {
        let side = side.min(self.channels - 1);
        self.prefix[last.min(self.frames) * self.channels + side]
            - self.prefix[first.min(self.frames) * self.channels + side]
    }
    fn wide_sum(&self, side: usize, first: f64, scale: f64) -> f64 {
        let cycles = (scale / self.period as f64).floor();
        let remainder = scale % self.period as f64;
        let mut sum = cycles * self.range(side, 0, self.frames) as f64;
        let mut cursor = first.rem_euclid(self.period as f64);
        let mut remaining = remainder;
        // A remainder visits at most one tail and one head of the period.
        for _ in 0..2 {
            let count = remaining.min(self.period as f64 - cursor);
            let last = cursor + count;
            let a = cursor.floor() as usize;
            let b = last.floor() as usize;
            if b > a {
                sum += self.range(side, a + 1, b) as f64;
                sum += self.range(side, a, a + 1) as f64 * (1. - cursor.fract());
                sum += self.range(side, b, b + 1) as f64 * last.fract();
            } else if count > 0. {
                sum += self.range(side, a, a + 1) as f64 * count;
            }
            remaining -= count;
            if remaining <= 0. {
                break;
            }
            cursor = 0.;
        }
        sum
    }
}

#[derive(Default)]
pub(crate) struct Cursor {
    chunk: usize,
    offset: f32,
    wide_offset: f64,
    remaining: usize,
}
impl Cursor {
    pub(crate) fn reset(&mut self) {
        self.remaining = 0;
    }
    pub(crate) fn sample(
        &mut self,
        pcm: &Pcm,
        prepared: &Prepared,
        source_offset: usize,
        rate: LoopDoppler,
    ) -> Sample {
        if self.remaining == 0 {
            let scaled = (source_offset as f32 * rate.previous_scale).trunc();
            let scaled = if scaled.is_finite() {
                f64::from(scaled)
            } else {
                (source_offset as f64 * f64::from(rate.previous_scale)).trunc()
            };
            self.wide_offset = scaled.rem_euclid(prepared.period as f64);
            self.chunk = (self.wide_offset / CHUNK as f64) as usize;
            self.offset = (self.wide_offset % CHUNK as f64) as f32;
            self.remaining = prepared.frames - source_offset;
        }
        self.remaining -= 1;
        if rate.scale > CHUNK as f32 {
            let scale = f64::from(rate.scale);
            let values = std::array::from_fn(|side| {
                (prepared.wide_sum(side, self.wide_offset, scale) / scale) as f32
            });
            self.wide_offset = (self.wide_offset + scale).rem_euclid(prepared.period as f64);
            return Sample {
                totals: values,
                count: 1.,
            };
        }
        let first = self.offset as usize;
        self.offset += rate.scale;
        let last = self.offset as usize;
        let mut totals = [0f32; 2];
        let channels = pcm.channels as usize;
        for source in first..last {
            if source == CHUNK {
                self.chunk = (self.chunk + 1) % (prepared.period / CHUNK);
                self.offset -= CHUNK as f32;
            }
            let frame = self.chunk * CHUNK + (source & (CHUNK - 1));
            if frame < prepared.frames {
                for (side, total) in totals.iter_mut().enumerate() {
                    *total += f32::from(pcm.samples[frame * channels + side.min(channels - 1)]);
                }
            }
        }
        Sample {
            totals,
            count: (last - first) as f32,
        }
    }
}
pub(crate) struct Sample {
    totals: [f32; 2],
    count: f32,
}
impl Sample {
    pub(crate) fn integer(&self, side: usize, gain: i64) -> i64 {
        ((self.totals[side] * gain as f32) / (256. * self.count)) as i32 as i64
    }
    pub(crate) fn floating(&self, side: usize, gain: f32) -> f32 {
        (self.totals[side] * gain) / self.count
    }
}
