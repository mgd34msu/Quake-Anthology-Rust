//! Rerelease brain RNG from `src/bots/behavior/rerelease/rng.ts`.
//!
//! The brain never rolls unseeded dice: every decision draws from an
//! injected generator, so a fixed seed replays a bot's whole match.
//! xorshift32 keeps one 32-bit word of state; construction discards
//! eight warm-up draws so small sequential seeds avalanche.

/// Warm-up draws discarded at construction.
pub const WARMUP_DRAWS: usize = 8;

/// Injected random source: a float in `[0, 1)`.
pub trait BotRandomT {
    /// Next unit draw.
    fn next(&mut self) -> f32;
}

/// xorshift32 generator.
#[derive(Debug, Clone, Copy)]
pub struct Xorshift32 {
    state: i32,
}

impl Xorshift32 {
    /// New generator; zero seeds nudge off the fixed point.
    #[must_use]
    pub fn new(seed: i32) -> Self {
        let mut generator = Self {
            state: if seed == 0 { 0x1a2b_3c4d } else { seed },
        };
        for _ in 0..WARMUP_DRAWS {
            generator.next();
        }
        generator
    }

    /// Current state word, for tests and checkpoints.
    #[must_use]
    pub fn peek(&self) -> i32 {
        self.state
    }

    /// Restore a checkpointed state word.
    pub fn restore(&mut self, state: i32) -> Result<(), crate::error::BotsError> {
        if state == 0 {
            return Err(crate::error::BotsError::BotCheckpoint(
                "bot random checkpoint must be a nonzero word".to_owned(),
            ));
        }
        self.state = state;
        Ok(())
    }
}

impl BotRandomT for Xorshift32 {
    fn next(&mut self) -> f32 {
        let mut x = self.state;
        x ^= x.wrapping_shl(13);
        x ^= (x as u32 >> 17) as i32;
        x ^= x.wrapping_shl(5);
        self.state = x;
        (x as u32) as f32 / 4_294_967_296.0
    }
}

impl crate::behavior::library::genetic::BotRandom for Xorshift32 {
    fn next_int(&mut self) -> i32 {
        self.next();
        self.state
    }

    fn next_unit(&mut self) -> f32 {
        self.next()
    }
}

/// A float in `[lo, hi)`.
#[must_use]
pub fn random_range(rng: &mut dyn BotRandomT, lo: f32, hi: f32) -> f32 {
    lo + rng.next() * (hi - lo)
}

/// An integer in `[0, count)`.
#[must_use]
pub fn random_index(rng: &mut dyn BotRandomT, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    let i = (rng.next() * count as f32) as usize;
    i.min(count - 1)
}

/// True with `percent` probability out of 100.
#[must_use]
pub fn random_chance(rng: &mut dyn BotRandomT, percent: f32) -> bool {
    if percent <= 0.0 {
        return false;
    }
    if percent >= 100.0 {
        return true;
    }
    rng.next() * 100.0 < percent
}
