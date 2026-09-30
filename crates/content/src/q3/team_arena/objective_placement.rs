//! Quake III team-arena: objective placement.
//!
//! Donor provenance: `src/content/q3/team-arena/objective-placement.ts`.

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::team_arena::mirrors::*;

// ---------------------------------------------------------------------------
// objective-placement.ts
// ---------------------------------------------------------------------------

/// Objective entity class name (`ObjectiveClassname`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectiveClassname {
    /// Red CTF flag.
    RedFlag,
    /// Blue CTF flag.
    BlueFlag,
    /// Neutral CTF flag.
    NeutralFlag,
    /// Red obelisk.
    RedObelisk,
    /// Blue obelisk.
    BlueObelisk,
    /// Neutral obelisk.
    NeutralObelisk,
}

impl ObjectiveClassname {
    /// Donor spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ObjectiveClassname::RedFlag => "team_CTF_redflag",
            ObjectiveClassname::BlueFlag => "team_CTF_blueflag",
            ObjectiveClassname::NeutralFlag => "team_CTF_neutralflag",
            ObjectiveClassname::RedObelisk => "team_redobelisk",
            ObjectiveClassname::BlueObelisk => "team_blueobelisk",
            ObjectiveClassname::NeutralObelisk => "team_neutralobelisk",
        }
    }
}

/// Objective placement record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectivePlacement {
    /// Entity class name.
    pub classname: Option<String>,
}

/// Objective placement check result (`ObjectivePlacementResult`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectivePlacementResult {
    /// All objectives present.
    Ready,
    /// Objectives missing.
    MissingObjectives {
        /// Missing class names.
        classnames: Vec<ObjectiveClassname>,
    },
    /// Mode unsupported by the product.
    UnsupportedMode {
        /// Product.
        product: Product,
        /// Game type code.
        game_type: i32,
    },
}

/// Check required objective placements (`checkObjectivePlacements`).
#[must_use]
pub fn check_objective_placements(
    product: Product,
    game_type: i32,
    placements: &[ObjectivePlacement],
) -> ObjectivePlacementResult {
    let required: &[ObjectiveClassname] = match game_type {
        game_type::FFA | game_type::TOURNAMENT | game_type::SINGLE_PLAYER | game_type::TEAM => &[],
        game_type::CTF => &[ObjectiveClassname::RedFlag, ObjectiveClassname::BlueFlag],
        game_type::ONE_FLAG_CTF => {
            if product != Product::MissionPack {
                return ObjectivePlacementResult::UnsupportedMode { product, game_type };
            }
            &[
                ObjectiveClassname::RedFlag,
                ObjectiveClassname::BlueFlag,
                ObjectiveClassname::NeutralFlag,
            ]
        }
        game_type::OBELISK => {
            if product != Product::MissionPack {
                return ObjectivePlacementResult::UnsupportedMode { product, game_type };
            }
            &[ObjectiveClassname::RedObelisk, ObjectiveClassname::BlueObelisk]
        }
        game_type::HARVESTER => {
            if product != Product::MissionPack {
                return ObjectivePlacementResult::UnsupportedMode { product, game_type };
            }
            &[
                ObjectiveClassname::RedObelisk,
                ObjectiveClassname::BlueObelisk,
                ObjectiveClassname::NeutralObelisk,
            ]
        }
        _ => return ObjectivePlacementResult::UnsupportedMode { product, game_type },
    };
    let missing: Vec<ObjectiveClassname> = required
        .iter()
        .filter(|classname| {
            !placements
                .iter()
                .any(|placement| placement.classname.as_deref() == Some(classname.as_str()))
        })
        .copied()
        .collect();
    if missing.is_empty() {
        ObjectivePlacementResult::Ready
    } else {
        ObjectivePlacementResult::MissingObjectives { classnames: missing }
    }
}
