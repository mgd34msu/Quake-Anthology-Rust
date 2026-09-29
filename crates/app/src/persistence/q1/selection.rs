//! Quake I save product selection ported from `src/persistence/q1-selection.ts`.
//!
//! Original version 5 saves do not identify `progs.dat` or the game
//! directory, so selection narrows installed Q1 products by map and
//! version 6 directories and refuses to guess between mods.

use super::super::PersistenceError;
use super::source_text::{Q1SaveData, Q1SaveFormat};

/// Installed product surface needed for save selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1SaveCatalogProduct {
    /// Product id.
    pub product_id: String,
    /// Expectation id (matched against the save directory name).
    pub expectation_id: String,
    /// Whether the product is a Quake I game.
    pub family_q1: bool,
    /// Whether installed content is present.
    pub installed: bool,
    /// Map paths (`maps/<name>.bsp`).
    pub maps: Vec<String>,
    /// Content directory.
    pub content_directory: String,
}

/// Installed catalog surface needed for save selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1SaveCatalog {
    /// Products.
    pub products: Vec<Q1SaveCatalogProduct>,
}

fn basename(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

fn dirname(path: &str) -> String {
    match path.rfind('/') {
        Some(index) => path[..index].to_string(),
        None => String::new(),
    }
}

fn valid_directory_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

/// Select the source game for an original save.
pub fn select_q1_save_product<'a>(
    catalog: &'a Q1SaveCatalog,
    save: &Q1SaveData,
    path: &str,
    selected: Option<&str>,
) -> Result<&'a Q1SaveCatalogProduct, PersistenceError> {
    let directory = basename(&dirname(path));
    let wanted = format!("maps/{}.bsp", save.map.to_lowercase());
    let mut candidates: Vec<&Q1SaveCatalogProduct> = catalog
        .products
        .iter()
        .filter(|product| {
            product.family_q1 && product.installed && product.maps.iter().any(|map| map.to_lowercase() == wanted)
        })
        .collect();
    if let Q1SaveFormat::V6 { game_directories } = &save.format {
        let directories: Vec<&str> = game_directories.split(';').filter(|part| !part.is_empty()).collect();
        let game = directories.last().map(|name| name.to_lowercase());
        match game {
            Some(game) if directories.iter().all(|name| valid_directory_name(name)) => {
                candidates.retain(|product| basename(&product.content_directory).to_lowercase() == game);
            }
            _ => {
                return Err(PersistenceError::BadSave(
                    "Invalid source save game directories.".to_string(),
                ));
            }
        }
    }
    if let Some(selected) = selected {
        return candidates
            .iter()
            .find(|product| product.expectation_id == selected || product.product_id == selected)
            .copied()
            .ok_or_else(|| PersistenceError::BadSave("Selected source game does not match this save.".to_string()));
    }
    let contextual: Vec<&&Q1SaveCatalogProduct> = candidates
        .iter()
        .filter(|product| product.expectation_id == directory)
        .collect();
    if contextual.len() == 1 {
        return Ok(contextual[0]);
    }
    if candidates.len() == 1 {
        return Ok(candidates[0]);
    }
    Err(PersistenceError::BadSave(
        if candidates.is_empty() {
            "Required source save game content is not installed."
        } else {
            "Select the source game for this save; its original format does not identify the program."
        }
        .to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn product(id: &str, maps: &[&str]) -> Q1SaveCatalogProduct {
        Q1SaveCatalogProduct {
            product_id: format!("{id}-product"),
            expectation_id: id.to_string(),
            family_q1: true,
            installed: true,
            maps: maps.iter().map(|map| map.to_string()).collect(),
            content_directory: format!("/games/{id}"),
        }
    }

    fn save() -> Q1SaveData {
        Q1SaveData {
            format: Q1SaveFormat::V5,
            comment: "x".to_string(),
            spawn_parameters: vec![0.0; 16],
            skill: 1,
            map: "e1m1".to_string(),
            time: 1.0,
            light_styles: vec!["m".to_string(); 64],
            globals: Vec::new(),
            entities: Vec::new(),
            extension_text: String::new(),
        }
    }

    #[test]
    fn selection_narrows_without_guessing() {
        let catalog = Q1SaveCatalog {
            products: vec![product("id1", &["maps/e1m1.bsp"]), product("rogue", &["maps/e1m1.bsp"])],
        };
        // Two candidates with no context: must ask.
        assert!(select_q1_save_product(&catalog, &save(), "/saves/slot.sav", None).is_err());
        // Save directory names the product.
        let picked = select_q1_save_product(&catalog, &save(), "/saves/rogue/slot.sav", None).unwrap();
        assert_eq!(picked.expectation_id, "rogue");
        // Explicit selection wins.
        let picked = select_q1_save_product(&catalog, &save(), "/saves/slot.sav", Some("id1")).unwrap();
        assert_eq!(picked.expectation_id, "id1");
        assert!(select_q1_save_product(&catalog, &save(), "/saves/slot.sav", Some("nope")).is_err());
        // Missing content reports as unavailable.
        let empty = Q1SaveCatalog { products: Vec::new() };
        assert!(select_q1_save_product(&empty, &save(), "/saves/slot.sav", None).is_err());
    }

    #[test]
    fn version_six_directories_filter_candidates() {
        let catalog = Q1SaveCatalog {
            products: vec![product("id1", &["maps/e1m1.bsp"]), product("rogue", &["maps/e1m1.bsp"])],
        };
        let mut save = save();
        save.format = Q1SaveFormat::V6 {
            game_directories: "id1;rogue".to_string(),
        };
        let picked = select_q1_save_product(&catalog, &save, "/saves/slot.sav", None).unwrap();
        assert_eq!(picked.expectation_id, "rogue");
        save.format = Q1SaveFormat::V6 {
            game_directories: "../evil".to_string(),
        };
        assert!(select_q1_save_product(&catalog, &save, "/saves/slot.sav", None).is_err());
    }
}
