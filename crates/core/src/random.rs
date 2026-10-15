//! Owned MT19937 and glibc TYPE_3 streams, selected at load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algorithm {
    Mt19937,
    GlibcType3,
}

#[expect(
    clippy::large_enum_variant,
    reason = "Inline state keeps cloned random checkpoints allocation-free."
)]
#[derive(Clone)]
enum State {
    Mt19937 {
        words: [u32; 624],
        index: usize,
    },
    GlibcType3 {
        words: [u32; 31],
        front: usize,
        rear: usize,
    },
}

#[derive(Clone)]
pub struct Random {
    state: State,
}

impl Random {
    pub fn new(seed: u32) -> Self {
        Self::with_algorithm(seed, Algorithm::Mt19937)
    }

    pub fn with_algorithm(seed: u32, algorithm: Algorithm) -> Self {
        match algorithm {
            Algorithm::Mt19937 => {
                let mut words = [0; 624];
                words[0] = seed;
                for i in 1..words.len() {
                    let previous = words[i - 1];
                    words[i] = 1812433253u32
                        .wrapping_mul(previous ^ (previous >> 30))
                        .wrapping_add(i as u32);
                }
                Self {
                    state: State::Mt19937 { words, index: 624 },
                }
            }
            Algorithm::GlibcType3 => {
                let mut words = [0; 31];
                let mut word = i64::from(if seed == 0 { 1 } else { seed } as i32);
                words[0] = word as u32;
                for value in &mut words[1..] {
                    word = 16807 * (word % 127773) - 2836 * (word / 127773);
                    if word < 0 {
                        word += 2147483647;
                    }
                    *value = word as u32;
                }
                let mut random = Self {
                    state: State::GlibcType3 {
                        words,
                        front: 3,
                        rear: 0,
                    },
                };
                for _ in 0..310 {
                    random.next_u32();
                }
                random
            }
        }
    }

    pub fn next_u32(&mut self) -> u32 {
        match &mut self.state {
            State::Mt19937 { words, index } => {
                if *index == words.len() {
                    for i in 0..words.len() {
                        let x = (words[i] & 0x80000000) | (words[(i + 1) % 624] & 0x7fffffff);
                        words[i] = words[(i + 397) % 624]
                            ^ (x >> 1)
                            ^ if x & 1 != 0 { 0x9908b0df } else { 0 };
                    }
                    *index = 0;
                }
                let mut y = words[*index];
                *index += 1;
                y ^= y >> 11;
                y ^= (y << 7) & 0x9d2c5680;
                y ^= (y << 15) & 0xefc60000;
                y ^ (y >> 18)
            }
            State::GlibcType3 { words, front, rear } => {
                words[*front] = words[*front].wrapping_add(words[*rear]);
                let value = words[*front] >> 1;
                *front = (*front + 1) % words.len();
                *rear = (*rear + 1) % words.len();
                value
            }
        }
    }
}
