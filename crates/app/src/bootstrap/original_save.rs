//! Original save import: shared passthrough or Q1 source restoration.
//!
//! Donor provenance: `src/app/bootstrap/original-save.ts`
//! (`prepareApplicationSave`). Save reading, Q1 product selection, skill
//! validation, and the restored-option rebuild are a direct port over the
//! persistence siblings. Content loading plus simulation restore/checkpoint
//! stay behind [`OriginalSaveRestorer`] (those donors are unported). The
//! stripped option fields that have no Rust counterpart
//! (`authoredCampaignStart`, `teamArenaSkirmish`, `q3MapLaunch`,
//! `q3Product`, `serverProfile`) need no removal.

use qa_core::identity::IdentityOwner;
use thiserror::Error;

use crate::options::{ApplicationOptions, GameFamily, GameMode, MatchRules, Network};
use crate::persistence::image::UnifiedSaveImage;
use crate::persistence::native_weapon::{NativeWeaponBehaviorDeclaration, WeaponBehaviorDefinition};
use crate::persistence::q1::selection::{select_q1_save_product, Q1SaveCatalog};
use crate::persistence::q1::source_text::Q1SaveData;
use crate::persistence::saved_game::{read_saved_game, SavedGame};
use crate::persistence::PersistenceError;

/// Failure of original save preparation.
#[derive(Debug, Error)]
pub enum OriginalSaveError {
    /// Save or option failure.
    #[error("{0}")]
    Save(String),
    /// Persistence failure.
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
    /// Identity failure.
    #[error(transparent)]
    Identity(#[from] qa_core::identity::IdentityError),
}

/// Prepared save image plus the options that produced it.
#[derive(Debug)]
pub struct PreparedApplicationSave {
    /// Save image to load.
    pub image: UnifiedSaveImage,
    /// Options for the save.
    pub options: ApplicationOptions,
}

/// Q1 source-save restoration: load content, restore into a simulation, checkpoint.
pub trait OriginalSaveRestorer {
    /// Restore Q1 source data under `options`, returning the checkpoint image.
    fn restore_q1_source(
        &mut self,
        options: &ApplicationOptions,
        data: &Q1SaveData,
        initial_source_milliseconds: f64,
        skill: u8,
    ) -> Result<UnifiedSaveImage, OriginalSaveError>;
}

/// Rebuild singleplayer Q1 options for an original save.
#[must_use]
pub fn restored_original_save_options(
    options: &ApplicationOptions,
    product_id: &str,
    map: &str,
    skill: u8,
) -> ApplicationOptions {
    let mut restored = options.clone();
    restored.movement_product = None;
    restored.map_product = None;
    restored.q2_game_library = None;
    restored.weapon_behavior = None;
    restored.bot_skill = None;
    restored.server_profile_path = None;
    restored.startup_commands = Vec::new();
    restored.explicit_rules = Default::default();
    restored.product = product_id.to_owned();
    restored.map = format!("maps/{map}.bsp");
    restored.skill = skill;
    restored.mode = GameMode::Singleplayer;
    restored.movement = GameFamily::Q1;
    restored.character = GameFamily::Q1;
    restored.character_model = "player".to_owned();
    restored.seats = 1;
    restored.rules = Some(MatchRules::Standard);
    restored.quake_c_program = Some("progs.dat".to_owned());
    restored.network = Network::Offline;
    restored
}

/// Prepare an original or shared save for loading.
pub fn prepare_application_save(
    options: &ApplicationOptions,
    catalog: &Q1SaveCatalog,
    path: &str,
    source_product: Option<&str>,
    legacy_native: &dyn Fn(&WeaponBehaviorDefinition) -> Option<NativeWeaponBehaviorDeclaration>,
    restorer: &mut dyn OriginalSaveRestorer,
) -> Result<PreparedApplicationSave, OriginalSaveError> {
    let saved = read_saved_game(path, legacy_native)?;
    if let SavedGame::Shared(image) = saved {
        return Ok(PreparedApplicationSave {
            image,
            options: options.clone(),
        });
    }
    let SavedGame::Q1Source(data) = saved else {
        return Err(OriginalSaveError::Save("Unsupported original save".to_owned()));
    };
    let product = select_q1_save_product(catalog, &data, path, source_product)?;
    if data.skill < 0 || data.skill > 3 {
        return Err(OriginalSaveError::Save("Original save skill must be 0..3".to_owned()));
    }
    let skill = data.skill as u8;
    let restored = restored_original_save_options(options, &product.product_id, &data.map, skill);
    let _identity = IdentityOwner::create("original-save-import")?;
    let image = restorer.restore_q1_source(&restored, &data, data.time * 1000.0, skill)?;
    Ok(PreparedApplicationSave {
        image,
        options: restored,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::q1::selection::Q1SaveCatalogProduct;
    use crate::persistence::q1::source_text::{encode_q1_save, Q1SaveFormat};

    struct FakeRestorer {
        skills: Vec<u8>,
        milliseconds: Vec<f64>,
    }

    impl OriginalSaveRestorer for FakeRestorer {
        fn restore_q1_source(
            &mut self,
            options: &ApplicationOptions,
            _data: &Q1SaveData,
            initial_source_milliseconds: f64,
            skill: u8,
        ) -> Result<UnifiedSaveImage, OriginalSaveError> {
            assert_eq!(options.mode, GameMode::Singleplayer);
            self.skills.push(skill);
            self.milliseconds.push(initial_source_milliseconds);
            let json = crate::persistence::recipe::tests_fixture_recipe_json();
            let recipe =
                crate::persistence::recipe::read_recipe(qa_world::save::value::SaveReader::new(&json), &|_| None)
                    .expect("recipe");
            Ok(UnifiedSaveImage {
                mods: None,
                legacy_armor_layout: false,
                recipe,
                frame: qa_core::time::FrameContext {
                    frame: 0,
                    time: qa_core::time::SourceTime::Milliseconds(0),
                    elapsed: qa_core::time::SourceTime::Milliseconds(0),
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
            })
        }
    }

    fn save_data() -> Q1SaveData {
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
                vec![crate::persistence::q1::source_text::QcTextPair {
                    key: "classname".to_string(),
                    value: "worldspawn".to_string(),
                }],
                vec![crate::persistence::q1::source_text::QcTextPair {
                    key: "classname".to_string(),
                    value: "player".to_string(),
                }],
            ],
            extension_text: String::new(),
        }
    }

    fn catalog() -> Q1SaveCatalog {
        Q1SaveCatalog {
            products: vec![Q1SaveCatalogProduct {
                product_id: "q1-id1".to_owned(),
                expectation_id: "id1".to_owned(),
                family_q1: true,
                installed: true,
                maps: vec!["maps/e1m1.bsp".to_owned()],
                content_directory: "id1".to_owned(),
            }],
        }
    }

    #[test]
    fn restores_q1_source_with_rebuilt_options() {
        let dir = std::env::temp_dir().join(format!("qa-orig-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        let path = dir.join("save.sav");
        std::fs::write(&path, encode_q1_save(&save_data()).expect("encode")).expect("write");
        let options = ApplicationOptions {
            product: "other".to_owned(),
            skill: 0,
            ..Default::default()
        };
        let mut restorer = FakeRestorer {
            skills: Vec::new(),
            milliseconds: Vec::new(),
        };
        let prepared = prepare_application_save(
            &options,
            &catalog(),
            path.to_str().expect("path"),
            None,
            &|_| None,
            &mut restorer,
        )
        .expect("prepare");
        assert_eq!(prepared.options.product, "q1-id1");
        assert_eq!(prepared.options.map, "maps/e1m1.bsp");
        assert_eq!(prepared.options.skill, 2);
        assert_eq!(prepared.options.mode, GameMode::Singleplayer);
        assert_eq!(prepared.options.network, Network::Offline);
        assert_eq!(restorer.skills, vec![2]);
        assert_eq!(restorer.milliseconds, vec![10_000.0]);
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn rejects_out_of_range_skill() {
        let mut data = save_data();
        data.skill = 5;
        let dir = std::env::temp_dir().join(format!("qa-orig-skill-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        let path = dir.join("save.sav");
        std::fs::write(&path, encode_q1_save(&data).expect("encode")).expect("write");
        let mut restorer = FakeRestorer {
            skills: Vec::new(),
            milliseconds: Vec::new(),
        };
        let err = prepare_application_save(
            &ApplicationOptions::default(),
            &catalog(),
            path.to_str().expect("path"),
            None,
            &|_| None,
            &mut restorer,
        )
        .expect_err("skill");
        assert!(restorer.skills.is_empty());
        assert!(matches!(err, OriginalSaveError::Persistence(_)));
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
