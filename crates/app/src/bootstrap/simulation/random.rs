//! Instance-owned source RNG: glibc TYPE_3 plus the Q2 rerelease MT19937.
//! glibc portion adapted from the Q3 donor; see donor `random.ts` header.

use qa_world::save::shared::SaveRandomState;
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
    pub fn checkpoint(&self) -> Mt19937Checkpoint {
        Mt19937Checkpoint {
            words: self.words,
            index: self.index,
            draws: self.draws,
        }
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

/// Convert a saved random stream to a restore checkpoint (C10).
///
/// Only the donor's restorable kinds convert; length/cast failures are
/// invalid states, exactly like the donor's `RangeError`s.
pub fn random_checkpoint_from_save(state: &SaveRandomState) -> Result<RandomCheckpoint, RandomError> {
    match state {
        SaveRandomState::GlibcRandom {
            words,
            front,
            rear,
            draws,
        } => {
            if words.len() != GLIBC_WORDS {
                return Err(RandomError::InvalidState);
            }
            let mut converted = [0i32; GLIBC_WORDS];
            for (slot, word) in converted.iter_mut().zip(words.iter()) {
                *slot = i32::try_from(*word).map_err(|_| RandomError::InvalidState)?;
            }
            Ok(RandomCheckpoint::Glibc(GlibcCheckpoint {
                words: converted,
                front: usize::try_from(*front).map_err(|_| RandomError::InvalidState)?,
                rear: usize::try_from(*rear).map_err(|_| RandomError::InvalidState)?,
                draws: *draws,
            }))
        }
        SaveRandomState::RereleaseMt19937 { words, index, draws } => {
            if words.len() != MT_WORDS {
                return Err(RandomError::InvalidState);
            }
            let mut converted = [0u32; MT_WORDS];
            converted.copy_from_slice(words);
            Ok(RandomCheckpoint::Mt19937(Box::new(Mt19937Checkpoint {
                words: converted,
                index: usize::try_from(*index).map_err(|_| RandomError::InvalidState)?,
                draws: *draws,
            })))
        }
        _ => Err(RandomError::InvalidState),
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
    fn save_checkpoint_round_trip() {
        use super::random_checkpoint_from_save;
        use qa_world::save::shared::SaveRandomState;
        let glibc = SaveRandomState::GlibcRandom {
            words: vec![7i64; super::GLIBC_WORDS],
            front: 1,
            rear: 2,
            draws: 9,
        };
        let checkpoint = random_checkpoint_from_save(&glibc).expect("glibc converts");
        match checkpoint {
            super::RandomCheckpoint::Glibc(state) => {
                assert_eq!(state.words, [7i32; super::GLIBC_WORDS]);
                assert_eq!((state.front, state.rear, state.draws), (1, 2, 9));
            }
            _ => panic!("glibc converts to glibc"),
        }
        let short = SaveRandomState::GlibcRandom {
            words: vec![7i64; 3],
            front: 0,
            rear: 0,
            draws: 0,
        };
        assert!(random_checkpoint_from_save(&short).is_err());
        let guest = SaveRandomState::Guest {
            module: "q3:game".to_string(),
            bytes: vec![1, 2, 3],
            draws: 0,
        };
        assert!(random_checkpoint_from_save(&guest).is_err());
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
