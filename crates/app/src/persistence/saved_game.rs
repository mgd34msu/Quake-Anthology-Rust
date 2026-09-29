//! Saved-game dispatch ported from `src/persistence/saved-game.ts`.
//!
//! Signature inspection selects the codec: `QT` opens a unified save,
//! anything else parses as an original Quake I source save. Malformed
//! shared saves never fall back to source parsing.

use super::image::{decode_save_image, encode_save_image, UnifiedSaveImage};
use super::native_weapon::{NativeWeaponBehaviorDeclaration, WeaponBehaviorDefinition};
use super::policy::write_contained_save;
use super::q1::source_text::{decode_q1_save, encode_q1_save, Q1SaveData};
use super::PersistenceError;

/// Decoded saved game.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum SavedGame {
    /// Unified shared save.
    Shared(UnifiedSaveImage),
    /// Original Quake I source save.
    Q1Source(Q1SaveData),
}

/// Decode a saved game from bytes.
pub fn decode_saved_game(
    bytes: &[u8],
    legacy_native: &dyn Fn(&WeaponBehaviorDefinition) -> Option<NativeWeaponBehaviorDeclaration>,
) -> Result<SavedGame, PersistenceError> {
    if bytes.first() == Some(&b'Q') && bytes.get(1) == Some(&b'T') {
        return decode_save_image(bytes, legacy_native).map(SavedGame::Shared);
    }
    let data = decode_q1_save(bytes)?;
    if !data
        .map
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'/' | b'-'))
        || data.map.is_empty()
        || data
            .map
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(PersistenceError::BadSave("q1.map: invalid source map name".to_string()));
    }
    if data.time < 0.0
        || data.skill < 0
        || data.skill > 3
        || data.entities.len() < 2
        || data.entities.first().is_some_and(Vec::is_empty)
    {
        return Err(PersistenceError::BadSave(
            "q1: save has no valid singleplayer world".to_string(),
        ));
    }
    Ok(SavedGame::Q1Source(data))
}

/// Encode a saved game.
pub fn encode_saved_game(save: &SavedGame) -> Result<Vec<u8>, PersistenceError> {
    match save {
        SavedGame::Shared(image) => Ok(encode_save_image(image)),
        SavedGame::Q1Source(data) => encode_q1_save(data),
    }
}

/// Read a saved game from disk.
pub fn read_saved_game(
    path: &str,
    legacy_native: &dyn Fn(&WeaponBehaviorDefinition) -> Option<NativeWeaponBehaviorDeclaration>,
) -> Result<SavedGame, PersistenceError> {
    decode_saved_game(&std::fs::read(path)?, legacy_native)
}

/// Write a saved game into a contained slot.
pub fn write_saved_game(directory: &str, path: &str, save: &SavedGame) -> Result<(), PersistenceError> {
    write_contained_save(directory, path, &encode_saved_game(save)?)
}

#[cfg(test)]
mod tests {
    use super::super::q1::source_text::Q1SaveFormat;
    use super::*;

    fn q1_save() -> Q1SaveData {
        Q1SaveData {
            format: Q1SaveFormat::V5,
            comment: "test".to_string(),
            spawn_parameters: vec![0.0; 16],
            skill: 2,
            map: "e1m1".to_string(),
            time: 10.0,
            light_styles: vec!["m".to_string(); 64],
            globals: Vec::new(),
            entities: vec![
                vec![super::super::q1::source_text::QcTextPair {
                    key: "classname".to_string(),
                    value: "worldspawn".to_string(),
                }],
                vec![super::super::q1::source_text::QcTextPair {
                    key: "classname".to_string(),
                    value: "player".to_string(),
                }],
            ],
            extension_text: String::new(),
        }
    }

    #[test]
    fn source_saves_dispatch_and_validate() {
        let bytes = encode_q1_save(&q1_save()).unwrap();
        assert!(matches!(
            decode_saved_game(&bytes, &|_| None).unwrap(),
            SavedGame::Q1Source(_)
        ));
        let mut bad = q1_save();
        bad.map = "../escape".to_string();
        assert!(decode_saved_game(&encode_q1_save(&bad).unwrap(), &|_| None).is_err());
        let mut bad = q1_save();
        bad.skill = 9;
        assert!(decode_saved_game(&encode_q1_save(&bad).unwrap(), &|_| None).is_err());
        let mut bad = q1_save();
        bad.entities.truncate(1);
        assert!(decode_saved_game(&encode_q1_save(&bad).unwrap(), &|_| None).is_err());
        // Malformed shared saves never fall back to source parsing.
        assert!(decode_saved_game(b"QT-garbage", &|_| None).is_err());
    }

    #[test]
    fn shared_saves_dispatch() {
        let json = crate::persistence::recipe::tests_fixture_recipe_json();
        let recipe =
            crate::persistence::recipe::read_recipe(qa_world::save::value::SaveReader::new(&json), &|_| None).unwrap();
        let image = UnifiedSaveImage {
            mods: None,
            legacy_armor_layout: false,
            recipe,
            frame: qa_core::time::FrameContext {
                frame: 0,
                time: qa_core::time::SourceTime::Milliseconds(0),
                elapsed: qa_core::time::SourceTime::Milliseconds(16),
                phase: qa_core::time::FramePhase::FrameEntry,
            },
            next_event_sequence: 0,
            clocks: Vec::new(),
            random: Vec::new(),
            actors: Vec::new(),
            bodies: Vec::new(),
            combat: Vec::new(),
            inventories: Vec::new(),
            configurations: Vec::new(),
            thinks: Vec::new(),
            providers: Vec::new(),
            guests: Vec::new(),
        };
        let bytes = encode_saved_game(&SavedGame::Shared(image)).unwrap();
        assert!(matches!(
            decode_saved_game(&bytes, &|_| None).unwrap(),
            SavedGame::Shared(_)
        ));
    }
}
