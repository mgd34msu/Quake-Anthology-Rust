//! Quake III team-arena: foreign objectives.
//!
//! Donor provenance: `src/content/q3/team-arena/foreign-objectives.ts`.

use crate::bsp::{parse_q1_entities, q1_entity_value, Q1Entity};
use qa_core::math::Vec3;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::team_arena::mirrors::*;
use crate::q3::team_arena::objective_placement::*;

// ---------------------------------------------------------------------------
// foreign-objectives.ts
// ---------------------------------------------------------------------------

/// World codec for objective adaptation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorldKind {
    /// Quake BSP.
    Q1Bsp,
    /// Quake II BSP.
    Q2Bsp,
    /// Quake III BSP.
    Q3Bsp,
}

/// World entity text for objective adaptation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignWorld {
    /// World codec.
    pub kind: WorldKind,
    /// Entity text.
    pub entities: String,
}

/// Explicit objective placement (`ExplicitObjectivePlacement`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExplicitObjectivePlacement {
    /// Objective class name.
    pub classname: ObjectiveClassname,
    /// Objective origin.
    pub origin: Vec3,
}

/// Foreign objective adaptation result (`ForeignObjectiveAdaptation`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForeignObjectiveAdaptation {
    /// Objectives ready.
    Ready {
        /// Adapted entity text.
        entities: String,
        /// Translated entity count.
        translated: usize,
    },
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

pub(crate) fn set_entity_field(entity: &Q1Entity, name: &str, value: &str) -> Q1Entity {
    let mut properties: Vec<(String, String)> = entity
        .properties
        .iter()
        .filter(|pair| pair.0 != name)
        .cloned()
        .collect();
    properties.push((name.to_string(), value.to_string()));
    Q1Entity { properties }
}

/// Translate foreign objectives into Q3 placements (`adaptForeignQ3Objectives`).
#[must_use]
pub fn adapt_foreign_q3_objectives(
    world: &ForeignWorld,
    product: Product,
    game_type: i32,
    explicit: &[ExplicitObjectivePlacement],
) -> ForeignObjectiveAdaptation {
    if world.kind == WorldKind::Q3Bsp && explicit.is_empty() {
        let parsed = parse_q1_entities(&world.entities, "objectives")
            .unwrap_or_else(|error| panic!("objective entities: {}", error.message));
        let placements: Vec<ObjectivePlacement> = parsed
            .iter()
            .map(|entity| ObjectivePlacement {
                classname: q1_entity_value(entity, "classname").map(str::to_string),
            })
            .collect();
        return match check_objective_placements(product, game_type, &placements) {
            ObjectivePlacementResult::Ready => ForeignObjectiveAdaptation::Ready {
                entities: world.entities.clone(),
                translated: 0,
            },
            ObjectivePlacementResult::MissingObjectives { classnames } => {
                ForeignObjectiveAdaptation::MissingObjectives { classnames }
            }
            ObjectivePlacementResult::UnsupportedMode { product, game_type } => {
                ForeignObjectiveAdaptation::UnsupportedMode { product, game_type }
            }
        };
    }
    let mut translated = 0usize;
    let mut entities: Vec<Q1Entity> = Vec::new();
    let obelisks = game_type == game_type::OBELISK || game_type == game_type::HARVESTER;
    let red_objective = if obelisks {
        "team_redobelisk"
    } else {
        "team_CTF_redflag"
    };
    let blue_objective = if obelisks {
        "team_blueobelisk"
    } else {
        "team_CTF_blueflag"
    };
    let aliases = [
        ("item_flag_team1", red_objective),
        ("item_flag_team2", blue_objective),
        ("info_player_team1", "team_CTF_redplayer"),
        ("info_player_team2", "team_CTF_blueplayer"),
        ("info_player_coop", "info_player_deathmatch"),
        ("info_player_start2", "info_player_deathmatch"),
        ("info_player_start", "info_player_deathmatch"),
    ];
    let parsed = parse_q1_entities(&world.entities, "objectives")
        .unwrap_or_else(|error| panic!("objective entities: {}", error.message));
    for entity in &parsed {
        let classname = q1_entity_value(entity, "classname").unwrap_or("");
        let replacement = if world.kind == WorldKind::Q3Bsp {
            None
        } else {
            aliases.iter().find(|pair| pair.0 == classname).map(|pair| pair.1)
        };
        let current = match replacement {
            Some(replacement) => set_entity_field(entity, "classname", replacement),
            None => entity.clone(),
        };
        entities.push(current.clone());
        if replacement.is_some() {
            translated += 1;
        }
        if replacement == Some("team_CTF_redplayer") || replacement == Some("team_CTF_blueplayer") {
            let spawn = if replacement == Some("team_CTF_redplayer") {
                "team_CTF_redspawn"
            } else {
                "team_CTF_bluespawn"
            };
            entities.push(set_entity_field(&current, "classname", spawn));
            translated += 1;
        }
    }
    for placement in explicit {
        if !placement.origin.x.is_finite() || !placement.origin.y.is_finite() || !placement.origin.z.is_finite() {
            panic!("Objective position must be finite");
        }
        if entities
            .iter()
            .any(|entity| q1_entity_value(entity, "classname") == Some(placement.classname.as_str()))
        {
            continue;
        }
        entities.push(Q1Entity {
            properties: vec![
                ("classname".to_string(), placement.classname.as_str().to_string()),
                (
                    "origin".to_string(),
                    format!("{} {} {}", placement.origin.x, placement.origin.y, placement.origin.z),
                ),
            ],
        });
    }
    let placements: Vec<ObjectivePlacement> = entities
        .iter()
        .map(|entity| ObjectivePlacement {
            classname: q1_entity_value(entity, "classname").map(str::to_string),
        })
        .collect();
    match check_objective_placements(product, game_type, &placements) {
        ObjectivePlacementResult::Ready => {}
        ObjectivePlacementResult::MissingObjectives { classnames } => {
            return ForeignObjectiveAdaptation::MissingObjectives { classnames };
        }
        ObjectivePlacementResult::UnsupportedMode { product, game_type } => {
            return ForeignObjectiveAdaptation::UnsupportedMode { product, game_type };
        }
    }
    if !entities.iter().any(|entity| {
        matches!(
            q1_entity_value(entity, "classname"),
            Some("info_player_deathmatch" | "team_CTF_redplayer" | "team_CTF_blueplayer")
        )
    }) {
        panic!("Q3 rules require an authored player spawn");
    }
    let quote = |value: &str| -> String {
        if value.contains('"') || value.contains('\0') {
            panic!("Invalid quoted entity field");
        }
        format!("\"{value}\"")
    };
    let text = entities
        .iter()
        .map(|entity| {
            let rows: Vec<String> = entity
                .properties
                .iter()
                .map(|row| format!("{} {}", quote(&row.0), quote(&row.1)))
                .collect();
            format!("{{\n{}\n}}\n", rows.join("\n"))
        })
        .collect::<String>();
    ForeignObjectiveAdaptation::Ready {
        entities: text,
        translated,
    }
}
