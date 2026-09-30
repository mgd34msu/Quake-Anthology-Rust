//! Monster animation declarations (`src/content/q1/base/animation.ts`).
//!
//! Copyright (C) 1996-2022 id Software LLC. GPL-2.0-or-later.

use crate::q1::foundation::types::{Q1Solid, Q1SoundChannel};
use crate::q1::{q1_error, Q1Error};

/// Monster steering mode (`MonsterAi`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MonsterAi {
    /// Stand (scan for targets).
    Stand,
    /// Walk (patrol).
    Walk,
    /// Run (combat).
    Run,
    /// Strafe while charging.
    ChargeSide,
    /// Strafe while attacking.
    MeleeSide,
    /// Charge the enemy.
    Charge,
    /// Melee attack.
    Melee,
    /// Pain knockback forward.
    Painforward,
    /// Turn toward the enemy.
    Turn,
    /// Face the enemy.
    Face,
    /// Pain knockback backward.
    Pain,
    /// Step forward.
    Forward,
}

impl MonsterAi {
    /// Donor mode text.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            MonsterAi::Stand => "stand",
            MonsterAi::Walk => "walk",
            MonsterAi::Run => "run",
            MonsterAi::ChargeSide => "charge_side",
            MonsterAi::MeleeSide => "melee_side",
            MonsterAi::Charge => "charge",
            MonsterAi::Melee => "melee",
            MonsterAi::Painforward => "painforward",
            MonsterAi::Turn => "turn",
            MonsterAi::Face => "face",
            MonsterAi::Pain => "pain",
            MonsterAi::Forward => "forward",
        }
    }

    /// Parse donor mode text.
    pub fn parse(text: &str) -> Result<Self, Q1Error> {
        match text {
            "stand" => Ok(MonsterAi::Stand),
            "walk" => Ok(MonsterAi::Walk),
            "run" => Ok(MonsterAi::Run),
            "charge_side" => Ok(MonsterAi::ChargeSide),
            "melee_side" => Ok(MonsterAi::MeleeSide),
            "charge" => Ok(MonsterAi::Charge),
            "melee" => Ok(MonsterAi::Melee),
            "painforward" => Ok(MonsterAi::Painforward),
            "turn" => Ok(MonsterAi::Turn),
            "face" => Ok(MonsterAi::Face),
            "pain" => Ok(MonsterAi::Pain),
            "forward" => Ok(MonsterAi::Forward),
            _ => Err(q1_error(format!("Unknown Q1 monster AI mode: {text}"))),
        }
    }
}

/// Sound chance comparison (`MonsterOperation["comparison"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SoundComparison {
    /// Play when the draw exceeds the chance.
    Greater,
    /// Play when the draw falls below the chance.
    Less,
}

impl SoundComparison {
    /// Donor comparison text.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            SoundComparison::Greater => "greater",
            SoundComparison::Less => "less",
        }
    }
}

/// One frame operation (`MonsterOperation`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MonsterOperation {
    /// Steering step.
    Ai {
        /// Steering mode.
        mode: MonsterAi,
        /// Step distance.
        distance: f64,
    },
    /// Conditional sound.
    Sound {
        /// Sound path.
        path: &'static str,
        /// Sound channel.
        channel: Q1SoundChannel,
        /// Distance attenuation.
        attenuation: f64,
        /// Chance comparison.
        comparison: SoundComparison,
        /// Play chance (`None` always plays).
        chance: Option<f64>,
    },
    /// Solidity change.
    Solid {
        /// New solidity.
        solid: Q1Solid,
    },
    /// Lightstyle change.
    Lightstyle {
        /// Light pattern.
        pattern: &'static str,
    },
    /// Named frame action.
    Action {
        /// Action name.
        name: &'static str,
    },
}

/// Named monster animation frame (`MonsterFrame`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonsterFrame {
    /// Model frame.
    pub frame: i32,
    /// Next frame name.
    pub next: &'static str,
    /// Frame operations in donor order.
    pub operations: &'static [MonsterOperation],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_modes_round_trip() {
        for mode in [
            MonsterAi::Stand,
            MonsterAi::Walk,
            MonsterAi::Run,
            MonsterAi::ChargeSide,
            MonsterAi::MeleeSide,
            MonsterAi::Charge,
            MonsterAi::Melee,
            MonsterAi::Painforward,
            MonsterAi::Turn,
            MonsterAi::Face,
            MonsterAi::Pain,
            MonsterAi::Forward,
        ] {
            assert_eq!(MonsterAi::parse(mode.as_str()), Ok(mode));
        }
        assert!(MonsterAi::parse("swim").is_err());
        assert_eq!(SoundComparison::Greater.as_str(), "greater");
    }
}
