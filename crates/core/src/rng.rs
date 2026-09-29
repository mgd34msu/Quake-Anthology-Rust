//! Owned `Q_rand` linear-congruential generator ported from
//! `src/core/numeric.ts` (`Q3Random`). State lives with the caller session
//! or module, never in a process global.

use thiserror::Error;

use crate::numeric::{q_crandom, q_rand, q_random};

/// Error for invalid generator seeds or draw counts.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RngError {
    /// Seeds must be signed 32-bit integers.
    #[error("Q3 random seed must be a signed 32-bit integer")]
    BadSeed,
    /// Draw counts must be finite and nonnegative.
    #[error("Invalid random draw count")]
    BadDraws,
}

/// Checkpoint of a [`Qrand`] generator: seed plus draw count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QrandCheckpoint {
    /// Current seed.
    pub seed: i32,
    /// Draws performed since construction.
    pub draws: u64,
}

/// Quake's `Q_rand` generator (`seed * 69069 + 1`, 32-bit wraparound).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Qrand {
    seed: i32,
    draws: u64,
}

impl Qrand {
    /// Create a generator with an explicit seed and draw count.
    pub fn new(seed: i32, draws: u64) -> Self {
        Self { seed, draws }
    }

    /// Create a generator, validating a wide seed.
    pub fn checked_new(seed: i64, draws: u64) -> Result<Self, RngError> {
        if seed < i64::from(i32::MIN) || seed > i64::from(i32::MAX) {
            return Err(RngError::BadSeed);
        }
        Ok(Self::new(seed as i32, draws))
    }

    /// Advance the generator and return the updated seed.
    pub fn next_integer(&mut self) -> i32 {
        self.seed = q_rand(self.seed);
        self.draws += 1;
        self.seed
    }

    /// Advance the generator and return Quake's `[0, 1)` fraction.
    pub fn next_unit(&mut self) -> f64 {
        q_random(self.take_seed()).value
    }

    /// Advance the generator and return Quake's `[-1, 1)` fraction.
    pub fn next_centered(&mut self) -> f64 {
        q_crandom(self.take_seed()).value
    }

    fn take_seed(&mut self) -> i32 {
        let seed = self.seed;
        self.seed = q_rand(seed);
        self.draws += 1;
        seed
    }

    /// Capture a restorable checkpoint.
    #[must_use]
    pub fn checkpoint(&self) -> QrandCheckpoint {
        QrandCheckpoint {
            seed: self.seed,
            draws: self.draws,
        }
    }

    /// Restore a checkpoint.
    #[must_use]
    pub fn restore(checkpoint: QrandCheckpoint) -> Self {
        Self {
            seed: checkpoint.seed,
            draws: checkpoint.draws,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qrand_matches_donor_sequence() {
        let mut rng = Qrand::new(1, 0);
        assert_eq!(rng.next_integer(), 69_070);
        assert_eq!(rng.next_integer(), 475_628_535);
        assert_eq!(
            rng.checkpoint(),
            QrandCheckpoint {
                seed: 475_628_535,
                draws: 2
            }
        );
        let mut restored = Qrand::restore(rng.checkpoint());
        assert_eq!(restored.next_integer(), rng.next_integer());
    }

    #[test]
    fn qrand_fractions_stay_in_range() {
        let mut rng = Qrand::new(-7, 0);
        for _ in 0..100 {
            assert!((0.0..1.0).contains(&rng.next_unit()));
            assert!((-1.0..1.0).contains(&rng.next_centered()));
        }
        assert_eq!(rng.checkpoint().draws, 200);
        assert!(Qrand::checked_new(2_147_483_648, 0).is_err());
    }
}
