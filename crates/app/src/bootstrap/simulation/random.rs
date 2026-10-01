//! Instance-owned source RNG: glibc TYPE_3 plus the Q2 rerelease MT19937.
//! glibc portion adapted from the Q3 donor; see donor `random.ts` header.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/core/random/q2-rerelease.ts`
//! (`Q2RereleaseRandom`, `Mt19937Checkpoint`, full STL-distribution API).
//! Donor: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/random.ts`.

use thiserror::Error;

/// Source RNG profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RandomProfile {
    /// glibc TYPE_3.
    Classic,
    /// Q2 rerelease MT19937.
    Q2Rerelease,
}

/// RNG failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RandomError {
    /// Invalid RNG checkpoint.
    #[error("Invalid source random state")]
    InvalidState,
    /// Checkpoint belongs to the other profile.
    #[error("Random checkpoint belongs to the other source profile")]
    ProfileMismatch,
    /// Empty integer range.
    #[error("Empty rerelease int32 range")]
    EmptyRange,
    /// Invalid float range.
    #[error("Invalid rerelease float range")]
    InvalidRange,
}

/// Pinned STL distribution for the rerelease stream.
pub const MT_DISTRIBUTION: &str = "msvc-2022-17.6";

const GLIBC_WORDS: usize = 31;
const GLIBC_WARMUP: usize = 310;
const MT_WORDS: usize = 624;
const MT_DEFAULT_SEED: u32 = 5489;
const UINT32_RANGE: f64 = 4_294_967_296.0;

/// glibc TYPE_3 checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlibcCheckpoint {
    /// State words.
    pub words: [i32; GLIBC_WORDS],
    /// Front cursor.
    pub front: usize,
    /// Rear cursor.
    pub rear: usize,
    /// Draw count.
    pub draws: u64,
}

/// MT19937 checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mt19937Checkpoint {
    /// State words.
    pub words: [u32; MT_WORDS],
    /// Read index.
    pub index: usize,
    /// Draw count.
    pub draws: u64,
}

impl Mt19937Checkpoint {
    /// Pinned distribution tag.
    #[must_use]
    pub fn distribution(&self) -> &'static str {
        MT_DISTRIBUTION
    }
}

/// Source RNG checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RandomCheckpoint {
    /// Classic stream.
    Glibc(GlibcCheckpoint),
    /// Rerelease stream.
    Mt19937(Box<Mt19937Checkpoint>),
}

/// Q2 rerelease MT19937 with the MSVC 2022 17.6 STL distributions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2RereleaseRandom {
    words: [u32; MT_WORDS],
    index: usize,
    draws: u64,
}

impl Default for Q2RereleaseRandom {
    fn default() -> Self {
        Self::new(MT_DEFAULT_SEED)
    }
}

impl Q2RereleaseRandom {
    /// Seed a new stream.
    #[must_use]
    pub fn new(seed: u32) -> Self {
        let mut words = [0u32; MT_WORDS];
        words[0] = seed;
        for i in 1..MT_WORDS {
            let previous = words[i - 1];
            words[i] = 1_812_433_253u32
                .wrapping_mul(previous ^ (previous >> 30))
                .wrapping_add(i as u32);
        }
        Self {
            words,
            index: MT_WORDS,
            draws: 0,
        }
    }

    /// Next raw word.
    pub fn next_u32(&mut self) -> u32 {
        if self.index == MT_WORDS {
            for i in 0..MT_WORDS {
                let joined = (self.words[i] & 0x8000_0000) | (self.words[(i + 1) % MT_WORDS] & 0x7fff_ffff);
                self.words[i] =
                    self.words[(i + 397) % MT_WORDS] ^ (joined >> 1) ^ if joined & 1 == 0 { 0 } else { 0x9908_b0df };
            }
            self.index = 0;
        }
        let mut value = self.words[self.index];
        self.index += 1;
        value ^= value >> 11;
        value ^= (value << 7) & 0x9d2c_5680;
        value ^= (value << 15) & 0xefc6_0000;
        value ^= value >> 18;
        self.draws += 1;
        value
    }

    /// Unit float; the whole word converts before division, unclamped.
    pub fn float(&mut self) -> f32 {
        (f64::from(self.next_u32() as f32) / UINT32_RANGE) as f32
    }

    /// Float in `[min, max]`.
    pub fn float_range(&mut self, min: f32, max: f32) -> Result<f32, RandomError> {
        if !min.is_finite() || !max.is_finite() || min > max {
            return Err(RandomError::InvalidRange);
        }
        Ok(self.float() * (max - min) + min)
    }

    /// Raw word as the unbounded integer draw.
    pub fn integer(&mut self) -> u32 {
        self.next_u32()
    }

    /// Integer in `[0, max)`; non-positive maxima yield 0.
    pub fn integer_below(&mut self, max: i32) -> Result<i32, RandomError> {
        if max <= 0 {
            return Ok(0);
        }
        self.integer_range(0, max)
    }

    /// Integer in `[min, max)`.
    pub fn integer_range(&mut self, min: i32, max: i32) -> Result<i32, RandomError> {
        if min >= max {
            return Err(RandomError::EmptyRange);
        }
        if min == max - 1 {
            return Ok(min);
        }
        let span = (i64::from(max) - i64::from(min)) as u128;
        Ok((i64::from(min) + self.offset(span) as i64) as i32)
    }

    /// 64-bit integer in `[min, max]`, bounds inclusive.
    pub fn integer64(&mut self, min: i64, max: i64) -> Result<i64, RandomError> {
        if min > max {
            return Err(RandomError::EmptyRange);
        }
        let span = (i128::from(max) - i128::from(min) + 1) as u128;
        Ok((i128::from(min) + self.offset(span) as i128) as i64)
    }

    /// Millisecond draw in `[min, max]`, bounds inclusive.
    pub fn time_milliseconds(&mut self, min: i64, max: i64) -> Result<i64, RandomError> {
        self.integer64(min, max)
    }

    fn offset(&mut self, range: u128) -> u128 {
        let bits = if range > (1u128 << 32) { 64 } else { 32 };
        let modulus = 1u128 << bits;
        let mask = modulus - 1;
        let threshold = (modulus - range) % range;
        loop {
            let mut sample = u128::from(self.next_u32());
            if bits == 64 {
                sample = (sample << 32) | u128::from(self.next_u32());
            }
            let product = sample * range;
            if (product & mask) >= threshold {
                return product >> bits;
            }
        }
    }

    /// Checkpoint the stream.
    #[must_use]
    pub fn capture(&self) -> Mt19937Checkpoint {
        self.checkpoint()
    }

    /// Checkpoint the stream.
    #[must_use]
    pub fn checkpoint(&self) -> Mt19937Checkpoint {
        Mt19937Checkpoint {
            words: self.words,
            index: self.index,
            draws: self.draws,
        }
    }

    /// Wire the engine stream to Q2 content: the engine owns the stream and
    /// content only draws through `Q2RereleaseRandomSource`.
    pub fn as_content_source(&mut self) -> Q2RereleaseContentSource<'_> {
        Q2RereleaseContentSource { random: self }
    }

    /// Restore a checkpoint.
    pub fn restore(&mut self, state: &Mt19937Checkpoint) -> Result<(), RandomError> {
        if state.index > MT_WORDS {
            return Err(RandomError::InvalidState);
        }
        self.words = state.words;
        self.index = state.index;
        self.draws = state.draws;
        Ok(())
    }
}

/// Engine-stream adapter implementing the content `Q2RereleaseRandomSource`
/// trait. Invalid ranges panic with the donor messages, matching the donor
/// throws; all in-range draws delegate draw-for-draw to the engine stream.
pub struct Q2RereleaseContentSource<'a> {
    random: &'a mut Q2RereleaseRandom,
}

impl qa_content::q2::support::misc::Q2RereleaseRandomSource for Q2RereleaseContentSource<'_> {
    fn next_uint32(&mut self) -> u32 {
        self.random.next_u32()
    }
    fn float_unit(&mut self) -> f32 {
        self.random.float()
    }
    fn float_max(&mut self, max_exclusive: f64) -> f32 {
        self.random
            .float_range(0.0, max_exclusive as f32)
            .expect("Invalid rerelease float range")
    }
    fn float_range(&mut self, min_inclusive: f64, max_exclusive: f64) -> f32 {
        self.random
            .float_range(min_inclusive as f32, max_exclusive as f32)
            .expect("Invalid rerelease float range")
    }
    fn integer_any(&mut self) -> i32 {
        self.random.integer() as i32
    }
    fn integer_max(&mut self, max_exclusive: i32) -> i32 {
        self.random
            .integer_below(max_exclusive)
            .expect("Invalid rerelease int32 range")
    }
    fn integer_range(&mut self, min_inclusive: i32, max_exclusive: i32) -> i32 {
        self.random
            .integer_range(min_inclusive, max_exclusive)
            .expect("Empty rerelease int32 range")
    }
    fn time_milliseconds(&mut self, min_inclusive: i64, max_inclusive: i64) -> i64 {
        self.random
            .time_milliseconds(min_inclusive, max_inclusive)
            .expect("Invalid rerelease int64 range")
    }
}

/// Instance-owned source RNG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRandom {
    rerelease: Option<Q2RereleaseRandom>,
    words: [i32; GLIBC_WORDS],
    front: usize,
    rear: usize,
    draws: u64,
}

impl SourceRandom {
    /// Seed a classic stream (donor default profile).
    #[must_use]
    pub fn new(seed: u32) -> Self {
        Self::with_profile(seed, RandomProfile::Classic)
    }

    /// Seed a stream for a profile.
    #[must_use]
    pub fn with_profile(seed: u32, profile: RandomProfile) -> Self {
        if profile == RandomProfile::Q2Rerelease {
            return Self {
                rerelease: Some(Q2RereleaseRandom::new(seed)),
                words: [0; GLIBC_WORDS],
                front: 0,
                rear: 0,
                draws: 0,
            };
        }
        let mut words = [0i32; GLIBC_WORDS];
        let mut word = (if seed == 0 { 1 } else { seed }) as i32;
        words[0] = word;
        for slot in words.iter_mut().skip(1) {
            word = 16807 * (word % 127_773) - 2836 * (word / 127_773);
            if word < 0 {
                word += 2_147_483_647;
            }
            *slot = word;
        }
        let mut random = Self {
            rerelease: None,
            words,
            front: 3,
            rear: 0,
            draws: 0,
        };
        for _ in 0..GLIBC_WARMUP {
            random.draw();
        }
        random
    }

    /// Rerelease stream, when profiled.
    #[must_use]
    pub fn rerelease(&mut self) -> Option<&mut Q2RereleaseRandom> {
        self.rerelease.as_mut()
    }

    /// Next integer draw.
    pub fn next_integer(&mut self) -> u32 {
        if let Some(rerelease) = &mut self.rerelease {
            return rerelease.next_u32();
        }
        self.draws += 1;
        self.draw()
    }

    /// Next unit draw.
    pub fn next_unit(&mut self) -> f32 {
        if let Some(rerelease) = &mut self.rerelease {
            return rerelease.float();
        }
        (f64::from(self.next_integer() & 0x7fff) / f64::from(0x7fff)) as f32
    }

    /// Checkpoint the stream.
    #[must_use]
    pub fn checkpoint(&self) -> RandomCheckpoint {
        if let Some(rerelease) = &self.rerelease {
            return RandomCheckpoint::Mt19937(Box::new(rerelease.checkpoint()));
        }
        RandomCheckpoint::Glibc(GlibcCheckpoint {
            words: self.words,
            front: self.front,
            rear: self.rear,
            draws: self.draws,
        })
    }

    /// Restore a checkpoint; cross-profile restores fail.
    pub fn restore(&mut self, state: &RandomCheckpoint) -> Result<(), RandomError> {
        match (state, &mut self.rerelease) {
            (RandomCheckpoint::Mt19937(state), Some(rerelease)) => rerelease.restore(state),
            (RandomCheckpoint::Glibc(state), None) => {
                if state.front >= GLIBC_WORDS || state.rear >= GLIBC_WORDS {
                    return Err(RandomError::InvalidState);
                }
                self.words = state.words;
                self.front = state.front;
                self.rear = state.rear;
                self.draws = state.draws;
                Ok(())
            }
            _ => Err(RandomError::ProfileMismatch),
        }
    }

    fn draw(&mut self) -> u32 {
        let sum = (self.words[self.front] as u32).wrapping_add(self.words[self.rear] as u32);
        self.words[self.front] = sum as i32;
        self.front = (self.front + 1) % GLIBC_WORDS;
        self.rear = (self.rear + 1) % GLIBC_WORDS;
        sum >> 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sequence(seed: u32, count: usize) -> Vec<u32> {
        let mut random = SourceRandom::new(seed);
        (0..count).map(|_| random.next_integer()).collect()
    }

    #[test]
    fn classic_stream_is_deterministic() {
        assert_eq!(sequence(1234, 8), sequence(1234, 8));
        assert_ne!(sequence(1, 8), sequence(2, 8));
    }

    #[test]
    fn classic_checkpoint_restores_draws() {
        let mut random = SourceRandom::new(99);
        for _ in 0..3 {
            random.next_integer();
        }
        let checkpoint = random.checkpoint();
        let before = [random.next_integer(), random.next_integer()];
        random.restore(&checkpoint).unwrap();
        let after = [random.next_integer(), random.next_integer()];
        assert_eq!(before, after);
    }

    #[test]
    fn cross_profile_restore_fails() {
        let mut classic = SourceRandom::new(7);
        let rerelease = SourceRandom::with_profile(7, RandomProfile::Q2Rerelease);
        assert_eq!(
            classic.restore(&rerelease.checkpoint()).unwrap_err(),
            RandomError::ProfileMismatch
        );
    }

    #[test]
    fn mt19937_matches_reference_vector() {
        let mut random = Q2RereleaseRandom::new(5489);
        assert_eq!(random.next_u32(), 3_499_211_612);
        assert_eq!(random.checkpoint().distribution(), "msvc-2022-17.6");
    }

    #[test]
    fn bounded_draws_hold_their_bounds() {
        let mut random = Q2RereleaseRandom::new(42);
        for _ in 0..16 {
            let unit = random.float();
            assert!((0.0..=1.0).contains(&unit));
        }
        assert_eq!(random.integer_below(1).unwrap(), 0);
        assert_eq!(random.integer_below(0).unwrap(), 0);
        assert_eq!(random.time_milliseconds(5, 5).unwrap(), 5);
    }

    #[test]
    fn capture_aliases_checkpoint_and_adapter_draws_through() {
        use qa_content::q2::support::misc::Q2RereleaseRandomSource;
        let random = Q2RereleaseRandom::new(7);
        assert_eq!(random.capture(), random.checkpoint());
        let mut direct = Q2RereleaseRandom::new(7);
        let mut adapted = Q2RereleaseRandom::new(7);
        {
            let mut source = adapted.as_content_source();
            assert_eq!(source.next_uint32(), direct.next_u32());
            assert_eq!(source.float_unit(), direct.float());
            assert_eq!(source.float_max(4.0), direct.float_range(0.0, 4.0).unwrap());
            assert_eq!(source.float_range(1.0, 3.0), direct.float_range(1.0, 3.0).unwrap());
            assert_eq!(source.integer_any(), direct.integer() as i32);
            assert_eq!(source.integer_max(9), direct.integer_below(9).unwrap());
            assert_eq!(source.integer_range(2, 9), direct.integer_range(2, 9).unwrap());
            assert_eq!(source.time_milliseconds(3, 9), direct.time_milliseconds(3, 9).unwrap());
        }
        assert_eq!(adapted.capture(), direct.capture());
    }

    #[test]
    fn classic_unit_draw_matches_donor_rounding() {
        let mut random = SourceRandom::new(5);
        let unit = random.next_unit();
        let mut raw = SourceRandom::new(5);
        let expected = (f64::from(raw.next_integer() & 0x7fff) / f64::from(0x7fff)) as f32;
        assert_eq!(unit, expected);
    }
}
