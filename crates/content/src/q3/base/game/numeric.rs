//! Quake III base/game: numeric.
//!
//! Donor provenance: `src/content/q3/base/game/numeric.ts`.

use qa_core::math::vec3;
use qa_core::math::Vec3;
use qa_core::numeric::q_rand;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::mirrors_game_state::*;

// ---------------------------------------------------------------------------
// numeric.ts: game numerics (donor `src/core/game-numeric.ts`, bg_lib.c)
// ---------------------------------------------------------------------------

/// Float scan result (`GameFloatScan`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GameFloatScan {
    /// Value.
    pub value: f32,
    /// Next offset.
    pub next_offset: usize,
}

/// Byte-string number cursor (`NumberInput`).
pub(crate) struct NumberInput {
    bytes: Vec<u8>,
    offset: usize,
}

impl NumberInput {
    fn new(text: &str, offset: usize) -> Result<Self, Q3GameError> {
        if offset > text.len() {
            return Err(range("Game number cursor is outside its backing string"));
        }
        let bytes = latin1_bytes(text).map_err(|_| range("Game numbers require byte characters"))?;
        if offset > bytes.len() {
            return Err(range("Game number cursor is outside its backing string"));
        }
        Ok(Self { bytes, offset })
    }

    fn byte(&self) -> Result<i32, Q3GameError> {
        if self.offset > self.bytes.len() {
            return Err(range("Game number scan reads beyond its backing string"));
        }
        if self.offset == self.bytes.len() {
            return Ok(0);
        }
        let byte = self.bytes[self.offset];
        Ok(if byte < 128 {
            i32::from(byte)
        } else {
            i32::from(byte) - 256
        })
    }

    fn take(&mut self) -> Result<i32, Q3GameError> {
        let byte = self.byte()?;
        self.offset += 1;
        Ok(byte)
    }

    fn skip_whitespace(&mut self) -> Result<(), Q3GameError> {
        while self.byte()? <= 32 && self.byte()? != 0 {
            self.offset += 1;
        }
        Ok(())
    }

    fn sign(&mut self) -> Result<i32, Q3GameError> {
        let byte = self.byte()?;
        if byte != 43 && byte != 45 {
            return Ok(1);
        }
        self.offset += 1;
        Ok(if byte == 45 { -1 } else { 1 })
    }
}

pub(crate) fn read_float(input: &mut NumberInput, scan: bool) -> Result<f32, Q3GameError> {
    input.skip_whitespace()?;
    if input.byte()? == 0 {
        return Ok(0.0);
    }
    let sign = input.sign()? as f32;
    let mut value = 0.0f32;
    let mut character = if scan { 48 } else { input.byte()? };
    if input.byte()? != 46 {
        loop {
            character = input.take()?;
            if !(48..=57).contains(&character) {
                break;
            }
            value = value * 10.0 + (character - 48) as f32;
        }
    } else if !scan {
        input.offset += 1;
    }
    if character == 46 {
        let mut fraction = 0.1f32;
        loop {
            character = input.take()?;
            if !(48..=57).contains(&character) {
                break;
            }
            value += (character - 48) as f32 * fraction;
            fraction *= 0.1;
        }
    }
    Ok(value * sign)
}

/// bg_lib atof (`gameAtof`).
pub fn game_atof(text: &str) -> Result<f32, Q3GameError> {
    read_float(&mut NumberInput::new(text, 0)?, false)
}

/// bg_lib atoi (`gameAtoi`).
pub fn game_atoi(text: &str) -> Result<i32, Q3GameError> {
    let mut input = NumberInput::new(text, 0)?;
    input.skip_whitespace()?;
    if input.byte()? == 0 {
        return Ok(0);
    }
    let sign = input.sign()?;
    let mut value = 0i32;
    loop {
        let character = input.take()?;
        if !(48..=57).contains(&character) {
            break;
        }
        value = value.wrapping_mul(10).wrapping_add(character - 48);
    }
    Ok(value.wrapping_mul(sign))
}

/// bg_lib float scan (`scanGameFloat`).
pub fn scan_game_float(text: &str, offset: usize) -> Result<GameFloatScan, Q3GameError> {
    let mut input = NumberInput::new(text, offset)?;
    let value = read_float(&mut input, true)?;
    Ok(GameFloatScan {
        value,
        next_offset: input.offset,
    })
}

/// QVM `sscanf("%f %f %f")` (`scanGameVector`).
pub fn scan_game_vector(text: &str) -> Result<Vec3, Q3GameError> {
    let mut input = NumberInput::new(text, 0)?;
    let x = read_float(&mut input, true)?;
    let y = read_float(&mut input, true)?;
    let z = read_float(&mut input, true)?;
    Ok(vec3(x, y, z))
}

/// Instance-owned bg_lib rand/srand and game random/crandom (`GameRandom`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameRandom {
    seed: i32,
}

impl GameRandom {
    /// New generator (`constructor`).
    pub fn new(seed: i32) -> Self {
        Self {
            seed: seed & 0x7fff_ffff | (seed & i32::MIN),
        }
    }

    /// Current seed.
    #[must_use]
    pub fn seed(&self) -> i32 {
        self.seed
    }

    /// Reset the seed (`reset`).
    pub fn reset(&mut self, seed: i32) {
        self.seed = seed;
    }

    /// bg_lib rand (`rand`).
    pub fn rand(&mut self) -> i32 {
        self.seed = q_rand(self.seed);
        self.seed & 0x7fff
    }

    /// Game random in `[0, 1]` (`random`).
    pub fn random(&mut self) -> f32 {
        self.rand() as f32 / f32::from(0x7fff_i16)
    }

    /// Game random in `[-1, 1]` (`crandom`).
    pub fn crandom(&mut self) -> f32 {
        2.0 * (self.random() - 0.5)
    }
}

impl Default for GameRandom {
    fn default() -> Self {
        Self::new(0)
    }
}
