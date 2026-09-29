//! Bot genetic selection from `src/bots/behavior/library/genetic.ts`
//! (`be_ai_gen.c`: `GeneticParentsAndChildSelection`).
//!
//! Interbreeding ranks bot characters by game score and picks two
//! parents plus a child slot by fitness-proportionate (roulette)
//! selection over the donor's 24-bit random draws.

/// Maximum ranked population.
pub const MAX_GENETIC_RANKS: usize = 128;

/// Random draws available to genetic selection.
pub trait BotRandom {
    /// Next 32-bit draw.
    fn next_int(&mut self) -> i32;
    /// Next unit draw in `[0, 1)`.
    fn next_unit(&mut self) -> f32;
}

/// Ranked population for selection.
#[derive(Debug, Clone)]
pub struct GeneticPopulation {
    /// Fitness ranks, best first.
    pub ranks: Vec<f32>,
}

impl GeneticPopulation {
    /// Build from ranks.
    #[must_use]
    pub fn new(ranks: Vec<f32>) -> Self {
        Self { ranks }
    }
}

/// Selected parent/child slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GeneticSelection {
    /// First parent slot.
    pub parent1: usize,
    /// Second parent slot.
    pub parent2: usize,
    /// Child slot.
    pub child: usize,
}

/// Roulette-select one rank by fitness proportion.
fn select_rank(ranks: &[f32], random: &mut dyn BotRandom) -> Option<usize> {
    let sum: f32 = ranks.iter().map(|rank| rank.max(0.0)).sum();
    if !(sum > 0.0) {
        return None;
    }
    let raw = random.next_int();
    let unit = ((raw & 0x00ff_ffff) as f32) / 16_777_215.0;
    let mut pick = unit * sum;
    for (index, rank) in ranks.iter().enumerate() {
        pick -= rank.max(0.0);
        if pick <= 0.0 {
            return Some(index);
        }
    }
    Some(ranks.len() - 1)
}

/// Select two parents and a child slot (`GeneticParentsAndChildSelection`).
///
/// Returns `None` when fewer than three ranks are valid; distinct parents
/// are drawn with replacement retries like the donor.
#[must_use]
pub fn genetic_parents_and_child_selection(
    population: &GeneticPopulation,
    random: &mut dyn BotRandom,
) -> Option<GeneticSelection> {
    let count = population.ranks.len().min(MAX_GENETIC_RANKS);
    if count < 3 {
        return None;
    }
    let ranks = &population.ranks[..count];
    let parent1 = select_rank(ranks, random)?;
    let mut parent2 = select_rank(ranks, random)?;
    for _ in 0..count {
        if parent2 != parent1 {
            break;
        }
        parent2 = select_rank(ranks, random)?;
    }
    let child = select_rank(ranks, random)?;
    Some(GeneticSelection {
        parent1,
        parent2,
        child,
    })
}

/// Deterministic xorshift32 draw source (donor `Xorshift32`).
#[derive(Debug, Clone, Copy)]
pub struct Xorshift32 {
    state: u32,
}

impl Xorshift32 {
    /// New generator; zero seeds fall back to the donor default.
    #[must_use]
    pub fn new(seed: u32) -> Self {
        Self {
            state: if seed == 0 { 0x2545_f491 } else { seed },
        }
    }

    /// Next raw draw.
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x.max(1);
        self.state
    }
}

impl BotRandom for Xorshift32 {
    fn next_int(&mut self) -> i32 {
        self.next_u32() as i32
    }

    fn next_unit(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / 16_777_216.0
    }
}
