//! QuakeC map entity text for non-Quake sources.
//!
//! Provenance: `src/app/bootstrap/simulation/quakec-map.ts`.

use qa_content::bsp::{parse_q1_entities, q1_entity_value, Q1Entity};
use qa_core::binary::BinaryError;

use super::types::{ApplicationWorld, SimulationMode};

/// QuakeC map entity failure.
#[derive(Debug, thiserror::Error)]
pub enum QuakeCMapError {
    /// Entity text does not parse.
    #[error(transparent)]
    Parse(#[from] BinaryError),
    /// Entity field cannot be quoted.
    #[error("QuakeC entity field cannot be quoted")]
    Unquotable,
}

fn quote(value: &str) -> Result<String, QuakeCMapError> {
    if value.contains('"') || value.contains('\0') {
        return Err(QuakeCMapError::Unquotable);
    }
    Ok(format!("\"{value}\""))
}

/// Keep authored module fields; only add an equivalent missing player-start role.
pub fn quake_c_map_entities(world: &ApplicationWorld, mode: SimulationMode) -> Result<String, QuakeCMapError> {
    if matches!(world, ApplicationWorld::Q1(_)) {
        return Ok(world.entities().to_string());
    }
    let mut entities = parse_q1_entities(world.entities(), "quakec-map")?;
    let wanted = if mode == SimulationMode::Deathmatch {
        "info_player_deathmatch"
    } else {
        "info_player_start"
    };
    if !entities
        .iter()
        .any(|entity| q1_entity_value(entity, "classname") == Some(wanted))
    {
        let candidates: &[&str] = if mode == SimulationMode::Deathmatch {
            &[
                "info_player_start",
                "info_player_coop",
                "team_CTF_redplayer",
                "team_CTF_blueplayer",
                "team_CTF_redspawn",
                "team_CTF_bluespawn",
            ]
        } else {
            &["info_player_deathmatch", "team_CTF_redplayer", "team_CTF_blueplayer"]
        };
        // The donor snapshots the list before appending replacements.
        let snapshot = entities.clone();
        for entity in &snapshot {
            if !candidates.contains(&q1_entity_value(entity, "classname").unwrap_or("")) {
                continue;
            }
            entities.push(Q1Entity {
                properties: entity
                    .properties
                    .iter()
                    .map(|(key, value)| {
                        if key == "classname" {
                            (key.clone(), wanted.to_string())
                        } else {
                            (key.clone(), value.clone())
                        }
                    })
                    .collect(),
            });
        }
    }
    let mut out = String::new();
    for entity in &entities {
        out.push_str("{\n");
        for (index, (key, value)) in entity.properties.iter().enumerate() {
            if index > 0 {
                out.push('\n');
            }
            out.push_str(&quote(key)?);
            out.push(' ');
            out.push_str(&quote(value)?);
        }
        out.push_str("\n}\n");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world_with_entities(entities: &str) -> ApplicationWorld<'static> {
        ApplicationWorld::Q3(qa_bots::scene::Q3WorldGeometry {
            entities: entities.to_string(),
            models: Vec::new(),
            vertices: Vec::new(),
            indices: Vec::new(),
            surfaces: Vec::new(),
            leaves: Vec::new(),
        })
    }

    #[test]
    fn keeps_q1_world_entities_verbatim() {
        let map = qa_content::bsp::Q1Map {
            format: qa_content::bsp::BspFormat::Bsp29,
            source: "test".to_string(),
            version: 29,
            data: &[],
            lumps: Vec::new(),
            entities: "verbatim".to_string(),
            entity_list: Vec::new(),
            planes: Vec::new(),
            vertices: Vec::new(),
            textures: Vec::new(),
            texture_offsets: Vec::new(),
            mip_offsets: Vec::new(),
            texture_info: Vec::new(),
            faces: Vec::new(),
            models: Vec::new(),
            nodes: Vec::new(),
            leaves: Vec::new(),
            edges: Vec::new(),
            clipnodes: Vec::new(),
            surface_edges: Vec::new(),
            leaf_faces: Vec::new(),
            visibility: &[],
            monochrome_lighting: &[],
            lighting: qa_content::bsp::BspLighting::Luminance8 { samples: &[] },
        };
        let world = ApplicationWorld::Q1(map);
        assert_eq!(
            quake_c_map_entities(&world, SimulationMode::Singleplayer).unwrap(),
            "verbatim"
        );
    }

    #[test]
    fn adds_missing_deathmatch_start_from_coop() {
        let world =
            world_with_entities("{\n\"classname\" \"worldspawn\"\n}\n{\n\"classname\" \"info_player_coop\"\n}\n");
        let text = quake_c_map_entities(&world, SimulationMode::Deathmatch).unwrap();
        assert!(text.contains("\"classname\" \"info_player_deathmatch\""));
        assert!(text.contains("\"classname\" \"info_player_coop\""));
    }

    #[test]
    fn keeps_existing_start() {
        let world = world_with_entities("{\n\"classname\" \"info_player_start\"\n}\n");
        let text = quake_c_map_entities(&world, SimulationMode::Singleplayer).unwrap();
        assert_eq!(text.matches("\"classname\" \"info_player_start\"").count(), 1);
    }

    #[test]
    fn rejects_unquotable_fields() {
        let world = world_with_entities("{\n\"classname\" \"info_player_start\"\n}\n");
        assert!(quake_c_map_entities(&world, SimulationMode::Singleplayer).is_ok());
        assert!(quote("a\"b").is_err());
        assert!(quote("a\0b").is_err());
    }
}
