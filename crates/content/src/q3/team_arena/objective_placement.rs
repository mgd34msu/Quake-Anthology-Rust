//! Quake III team-arena: objective placement.
//!
//! Donor provenance: `src/content/q3/team-arena/objective-placement.ts`.

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::shared::definitions::*;

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

// Pattern-position aliases for GameType discriminants (`as` casts are not patterns).
const GT_FFA: i32 = GameType::GtFfa as i32;
const GT_TOURNAMENT: i32 = GameType::GtTournament as i32;
const GT_SINGLE_PLAYER: i32 = GameType::GtSinglePlayer as i32;
const GT_TEAM: i32 = GameType::GtTeam as i32;
const GT_CTF: i32 = GameType::GtCtf as i32;
const GT_ONE_FLAG_CTF: i32 = GameType::Gt1fctf as i32;
const GT_OBELISK: i32 = GameType::GtObelisk as i32;
const GT_HARVESTER: i32 = GameType::GtHarvester as i32;

/// Check required objective placements (`checkObjectivePlacements`).
#[must_use]
pub fn check_objective_placements(
    product: Product,
    game_type: i32,
    placements: &[ObjectivePlacement],
) -> ObjectivePlacementResult {
    let required: &[ObjectiveClassname] = match game_type {
        GT_FFA | GT_TOURNAMENT | GT_SINGLE_PLAYER | GT_TEAM => &[],
        GT_CTF => &[ObjectiveClassname::RedFlag, ObjectiveClassname::BlueFlag],
        GT_ONE_FLAG_CTF => {
            if product != Product::Missionpack {
                return ObjectivePlacementResult::UnsupportedMode { product, game_type };
            }
            &[
                ObjectiveClassname::RedFlag,
                ObjectiveClassname::BlueFlag,
                ObjectiveClassname::NeutralFlag,
            ]
        }
        GT_OBELISK => {
            if product != Product::Missionpack {
                return ObjectivePlacementResult::UnsupportedMode { product, game_type };
            }
            &[ObjectiveClassname::RedObelisk, ObjectiveClassname::BlueObelisk]
        }
        GT_HARVESTER => {
            if product != Product::Missionpack {
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
