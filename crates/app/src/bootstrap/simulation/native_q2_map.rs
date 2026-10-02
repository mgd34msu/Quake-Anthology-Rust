//! Equivalent authored entity roles for native Quake II maps.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/native-q2-map.ts`.
//!
//! Geometry stays in shared scene queries; only equivalent authored source
//! entity roles change. The public entry adapts [`ApplicationWorld`] to the
//! donor `NativeQ2MapGeometry` projections (`kind`, `entities`, model count)
//! and every rule below them is donor logic.

use qa_content::bsp::{parse_q1_entities, q1_entity_value, Q1Entity};
use qa_content::q2::foundation::host::Q2Edition;
use qa_core::binary::BinaryError;
use thiserror::Error;

use super::types::{ApplicationWorld, SimulationMode};

/// Native Quake II map preparation failure.
#[derive(Debug, Error)]
pub enum NativeQ2MapError {
    /// Inline models exceed the source capacity (donor `RangeError`).
    #[error("Native Quake II map exceeds source inline-model capacity")]
    TooManyModels {
        /// Decoded model count.
        count: usize,
        /// Edition capacity.
        maximum: usize,
    },
    /// Entity text does not parse.
    #[error(transparent)]
    Entities(#[from] BinaryError),
    /// An entity field cannot be quoted (donor `Error`).
    #[error("Native Quake II entity field cannot be quoted")]
    UnquotableField,
}

/// Quake III CTF roles and their native Quake II equivalents.
const Q3_CTF_ALIASES: [(&str, &str); 6] = [
    ("team_CTF_redflag", "item_flag_team1"),
    ("team_CTF_blueflag", "item_flag_team2"),
    ("team_CTF_redplayer", "info_player_team1"),
    ("team_CTF_redspawn", "info_player_team1"),
    ("team_CTF_blueplayer", "info_player_team2"),
    ("team_CTF_bluespawn", "info_player_team2"),
];

/// Inline-model capacity by edition (donor `maximum`).
#[must_use]
pub fn inline_model_capacity(edition: Q2Edition) -> usize {
    match edition {
        Q2Edition::Classic => 255,
        Q2Edition::Rerelease => 8190,
    }
}

/// Decoded model count behind an application world.
fn model_count(world: &ApplicationWorld) -> usize {
    match world {
        ApplicationWorld::Q1(map) => map.models.len(),
        ApplicationWorld::Q2(decoded) => decoded.models.len(),
        ApplicationWorld::Q3(geometry) => geometry.models.len(),
    }
}

fn replace_classname(entity: &Q1Entity, classname: &str) -> Q1Entity {
    Q1Entity {
        properties: entity
            .properties
            .iter()
            .map(|(key, value)| {
                if key == "classname" {
                    (key.clone(), classname.to_string())
                } else {
                    (key.clone(), value.clone())
                }
            })
            .collect(),
    }
}

fn quote(value: &str) -> Result<String, NativeQ2MapError> {
    if value.contains('"') || value.contains('\0') {
        return Err(NativeQ2MapError::UnquotableField);
    }
    Ok(format!("\"{value}\""))
}

/// Prepare native entity text from decoded-world projections.
///
/// This is the donor function body over the `NativeQ2MapGeometry`
/// projections; [`prepare_native_q2_map`] supplies them from an
/// [`ApplicationWorld`].
fn prepare_from_parts(
    kind: &str,
    entities: &str,
    models: usize,
    edition: Q2Edition,
    mode: SimulationMode,
) -> Result<String, NativeQ2MapError> {
    let maximum = inline_model_capacity(edition);
    if models > maximum {
        return Err(NativeQ2MapError::TooManyModels { count: models, maximum });
    }
    if kind == "q2-bsp" {
        return Ok(entities.to_string());
    }
    let parsed = parse_q1_entities(entities, "native-q2-map")?;
    let mut rewritten: Vec<Q1Entity> = parsed
        .iter()
        .map(|entity| {
            let classname = q1_entity_value(entity, "classname").unwrap_or("");
            let replacement = if kind == "q3-bsp" {
                Q3_CTF_ALIASES
                    .iter()
                    .find(|(alias, _)| *alias == classname)
                    .map(|(_, replacement)| *replacement)
            } else {
                None
            };
            replacement.map_or_else(|| entity.clone(), |next| replace_classname(entity, next))
        })
        .collect();
    let deathmatch = mode == SimulationMode::Deathmatch;
    let wanted = if deathmatch {
        "info_player_deathmatch"
    } else {
        "info_player_start"
    };
    if !rewritten
        .iter()
        .any(|entity| q1_entity_value(entity, "classname") == Some(wanted))
    {
        let equivalents: &[&str] = if deathmatch {
            &[
                "info_player_start",
                "info_player_coop",
                "info_player_team1",
                "info_player_team2",
            ]
        } else {
            &["info_player_deathmatch"]
        };
        // Donor iterates a snapshot while pushing, so duplicates of
        // duplicates never occur.
        for entity in rewritten.clone() {
            if equivalents.contains(&q1_entity_value(&entity, "classname").unwrap_or("")) {
                rewritten.push(replace_classname(&entity, wanted));
            }
        }
    }
    let mut out = String::new();
    for entity in &rewritten {
        out.push_str("{\n");
        let mut first = true;
        for (key, value) in &entity.properties {
            if !first {
                out.push('\n');
            }
            first = false;
            out.push_str(&quote(key)?);
            out.push(' ');
            out.push_str(&quote(value)?);
        }
        out.push_str("\n}\n");
    }
    Ok(out)
}

/// Prepare equivalent authored entity roles for a native Quake II map.
///
/// Quake II geometry passes through untouched; Quake and Quake III geometry
/// gains CTF aliases (Quake III only) and a guaranteed spawn role for the
/// selected mode.
pub fn prepare_native_q2_map(
    world: &ApplicationWorld,
    edition: Q2Edition,
    mode: SimulationMode,
) -> Result<String, NativeQ2MapError> {
    prepare_from_parts(world.kind(), world.entities(), model_count(world), edition, mode)
}

#[cfg(test)]
mod tests {
    use qa_bots::scene::Q3WorldGeometry;
    use qa_content::bsp::{BspFormat, BspLighting, Q1Map};
    use qa_content::common::Bounds;

    use super::*;
    use qa_content::bsp::WorldModel;

    fn alliance_entities() -> &'static str {
        "{\n\"classname\" \"worldspawn\"\n}\n{\n\"classname\" \"team_CTF_redflag\"\n\"origin\" \"1 2 3\"\n}\n{\n\"classname\" \"info_player_start\"\n}\n"
    }

    fn q3_world(entities: &str) -> ApplicationWorld<'static> {
        ApplicationWorld::Q3(Q3WorldGeometry {
            entities: entities.to_string(),
            models: Vec::new(),
            vertices: Vec::new(),
            indices: Vec::new(),
            surfaces: Vec::new(),
            leaves: Vec::new(),
        })
    }

    fn q1_world(entities: &str, models: Vec<WorldModel>) -> ApplicationWorld<'static> {
        ApplicationWorld::Q1(Q1Map {
            format: BspFormat::Bsp29,
            source: "test".to_string(),
            version: 29,
            data: b"",
            lumps: Vec::new(),
            entities: entities.to_string(),
            entity_list: Vec::new(),
            planes: Vec::new(),
            vertices: Vec::new(),
            textures: Vec::new(),
            texture_offsets: Vec::new(),
            mip_offsets: Vec::new(),
            texture_info: Vec::new(),
            faces: Vec::new(),
            models,
            nodes: Vec::new(),
            leaves: Vec::new(),
            edges: Vec::new(),
            clipnodes: Vec::new(),
            surface_edges: Vec::new(),
            leaf_faces: Vec::new(),
            visibility: b"",
            monochrome_lighting: b"",
            lighting: BspLighting::Luminance8 { samples: b"" },
        })
    }

    fn world_model() -> WorldModel {
        WorldModel {
            bounds: Bounds {
                min: [0.0, 0.0, 0.0],
                max: [1.0, 1.0, 1.0],
            },
            origin: [0.0, 0.0, 0.0],
            headnodes: [0, 0, 0, 0],
            visible_leaves: 0,
            face_first: 0,
            face_count: 0,
        }
    }

    #[test]
    fn q2_geometry_passes_through_untouched() {
        let entities = "{\n\"classname\" \"team_CTF_redflag\"\n}\n";
        let out = prepare_from_parts("q2-bsp", entities, 3, Q2Edition::Classic, SimulationMode::Deathmatch).unwrap();
        assert_eq!(out, entities);
    }

    #[test]
    fn inline_model_capacity_matches_edition() {
        assert_eq!(inline_model_capacity(Q2Edition::Classic), 255);
        assert_eq!(inline_model_capacity(Q2Edition::Rerelease), 8190);
        let error =
            prepare_from_parts("q1-bsp", "", 256, Q2Edition::Classic, SimulationMode::Singleplayer).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Native Quake II map exceeds source inline-model capacity"
        );
        assert!(matches!(
            error,
            NativeQ2MapError::TooManyModels {
                count: 256,
                maximum: 255
            }
        ));
        assert!(prepare_from_parts("", "", 8190, Q2Edition::Rerelease, SimulationMode::Singleplayer).is_ok());
    }

    #[test]
    fn q3_aliases_apply_only_to_quake_iii() {
        let out = prepare_from_parts(
            "q3-bsp",
            alliance_entities(),
            0,
            Q2Edition::Rerelease,
            SimulationMode::Singleplayer,
        )
        .unwrap();
        assert!(out.contains("\"classname\" \"item_flag_team1\""), "{out}");
        assert!(!out.contains("team_CTF_redflag"), "{out}");
        let untouched = prepare_from_parts(
            "q1-bsp",
            alliance_entities(),
            0,
            Q2Edition::Rerelease,
            SimulationMode::Singleplayer,
        )
        .unwrap();
        assert!(untouched.contains("team_CTF_redflag"), "{untouched}");
    }

    #[test]
    fn deathmatch_duplicates_start_spawns() {
        let out = prepare_from_parts(
            "q1-bsp",
            alliance_entities(),
            0,
            Q2Edition::Classic,
            SimulationMode::Deathmatch,
        )
        .unwrap();
        assert!(out.contains("\"classname\" \"info_player_deathmatch\""), "{out}");
        assert!(out.contains("\"classname\" \"info_player_start\""), "{out}");
    }

    #[test]
    fn singleplayer_keeps_existing_start() {
        let out = prepare_from_parts(
            "q1-bsp",
            alliance_entities(),
            0,
            Q2Edition::Classic,
            SimulationMode::Coop,
        )
        .unwrap();
        assert_eq!(out.matches("\"classname\" \"info_player_start\"").count(), 1, "{out}");
    }

    #[test]
    fn unquotable_fields_fail() {
        // The tokenizer cannot produce quotes or NUL (matching the donor
        // parser, which has no escapes), so the defensive check is covered
        // directly.
        assert_eq!(quote("clean").unwrap(), "\"clean\"");
        assert!(matches!(quote("a\"b").unwrap_err(), NativeQ2MapError::UnquotableField));
        assert!(matches!(quote("a\0b").unwrap_err(), NativeQ2MapError::UnquotableField));
    }

    #[test]
    fn malformed_entities_fail() {
        let error = prepare_from_parts(
            "q1-bsp",
            "{ \"classname\" ",
            0,
            Q2Edition::Classic,
            SimulationMode::Singleplayer,
        )
        .unwrap_err();
        assert!(matches!(error, NativeQ2MapError::Entities(_)));
    }

    #[test]
    fn application_worlds_route_by_kind() {
        let world = q3_world(alliance_entities());
        let out = prepare_native_q2_map(&world, Q2Edition::Rerelease, SimulationMode::Deathmatch).unwrap();
        assert!(out.contains("\"classname\" \"item_flag_team1\""), "{out}");
        assert!(out.contains("\"classname\" \"info_player_deathmatch\""), "{out}");

        let world = q1_world(alliance_entities(), vec![world_model()]);
        let out = prepare_native_q2_map(&world, Q2Edition::Classic, SimulationMode::Singleplayer).unwrap();
        assert!(out.contains("team_CTF_redflag"), "{out}");

        let world = q1_world("", vec![world_model(); 256]);
        let error = prepare_native_q2_map(&world, Q2Edition::Classic, SimulationMode::Singleplayer).unwrap_err();
        assert!(matches!(
            error,
            NativeQ2MapError::TooManyModels {
                count: 256,
                maximum: 255
            }
        ));
    }
}
