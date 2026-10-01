//! Application bot asset loading.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/bot-assets.ts`.
//!
//! Missing siblings: `LoadedApplicationContent` (`content.ts`, content
//! partition) and `SharedSimulation` (`runtime.ts`, runtime partition). The
//! [`BotAssetContent`] and [`BotAssetSimulation`] seams expose exactly the
//! donor's content and simulation surface; the partitions implement them
//! post-merge. The donor is async; bootstrap ports are sync.

use qa_bots::behavior::assets::{BotAssetFiles, BotSourceFiles};
use qa_bots::behavior::rerelease::data::knowledge::BotKnowledge;
use qa_bots::behavior::rerelease::data::knowledge_q1::load_quake1_knowledge;
use qa_bots::behavior::rerelease::data::knowledge_q2::{bot_load_knowledge, SettingsPlatform};
use qa_bots::error::BotsError;
use qa_client::text::localization::{LocalizationProfile, LocalizationTable};
use qa_client::text::resources::{load_server_localization_resources, LocalizationReader};
use thiserror::Error;

use super::bot_selected_knowledge::native_q3_weapon_knowledge;
use super::bot_selected_knowledge::BotKnowledgeError as SelectedKnowledgeError;

/// Rerelease bot source family (donor `source` spellings).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RereleaseBotSource {
    /// Q1 rerelease.
    Q1,
    /// Q2 rerelease.
    Q2,
}

impl RereleaseBotSource {
    /// Donor spelling.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            RereleaseBotSource::Q1 => "q1-rerelease",
            RereleaseBotSource::Q2 => "q2-rerelease",
        }
    }
}

/// Application bot assets (donor `ApplicationBotAssets`).
#[derive(Debug, Clone)]
pub enum ApplicationBotAssets {
    /// Q3 bot assets.
    Q3 {
        /// Bot source files.
        files: BotAssetFiles,
    },
    /// Rerelease bot assets.
    Rerelease {
        /// Source family.
        source: RereleaseBotSource,
        /// Bot source files.
        files: BotAssetFiles,
        /// Parsed knowledge.
        knowledge: Box<BotKnowledge>,
        /// Server localization.
        localization: LocalizationTable,
    },
}

/// Selected weapon recipe entry (donor `recipe.weapons[0]` shape).
#[derive(Debug, Clone, PartialEq)]
pub struct BotAssetWeapon {
    /// Provider id.
    pub provider: String,
    /// Content id.
    pub content: String,
}

/// Content surface for bot asset loading.
pub trait BotAssetContent {
    /// Donor `content.recipe.map.entities.content`.
    fn map_entities_content(&self) -> String;
    /// Donor `content.recipe.weapons[0]`.
    fn selected_weapon(&self) -> Option<BotAssetWeapon>;
    /// Donor `loadMountedBotAssetFiles(await content.forContent(id), catalog)`.
    fn load_bot_files(&self, content: &str) -> BotAssetFiles;
    /// Donor installed sibling product content for an expectation id.
    fn installed_sibling_content(&self, expectation: &str) -> Option<String>;
    /// Donor `content.catalog.product("q3-baseq3").id`.
    fn q3_base_content(&self) -> String;
    /// Donor `(await mounts.open(path))?.bytes`.
    fn open_content_file(&self, content: &str, path: &str) -> Option<Vec<u8>>;
}

/// Simulation surface for bot asset loading.
pub trait BotAssetSimulation {
    /// Donor `simulation.q3Source() !== null || simulation.q3Guest() !== null`.
    fn has_q3_source_or_guest(&self) -> bool;
    /// Donor `simulation.q1Source() !== null`.
    fn has_q1_source(&self) -> bool;
    /// Donor `(simulation.q1Source()?.cvars ?? simulation.q2ServerCvars())`
    /// `language` variable, or `None` without a registry.
    fn bot_language(&self) -> Option<String>;
}

/// Bot asset loading failures.
#[derive(Debug, Error)]
pub enum BotAssetError {
    /// Native bot definitions disappeared during loading.
    #[error("Native bot definitions disappeared during loading")]
    Disappeared,
    /// Invalid native bot definitions.
    #[error("Invalid native bot definitions: {0}")]
    Invalid(String),
    /// Selected arsenal lacks authored bot weapon knowledge.
    #[error("Selected arsenal lacks authored bot weapon knowledge")]
    LacksArsenal,
    /// Knowledge parsing failure.
    #[error(transparent)]
    Knowledge(#[from] BotsError),
    /// Selected Q3 weapon knowledge failure.
    #[error(transparent)]
    Selected(#[from] SelectedKnowledgeError),
}

struct MountReader<'a, C> {
    content: &'a C,
    mounts: String,
    native: &'a mut BotAssetFiles,
}

impl<C: BotAssetContent> LocalizationReader for MountReader<'_, C> {
    fn read(&mut self, path: &str) -> Option<Vec<u8>> {
        let bytes = self.content.open_content_file(&self.mounts, path);
        if let Some(bytes) = bytes.as_ref() {
            self.native.add(path, bytes);
        }
        bytes
    }
}

/// Authored map bot definitions win. Classic products without these files
/// keep their existing policy.
pub fn load_application_bot_assets<C: BotAssetContent, S: BotAssetSimulation>(
    content: &C,
    simulation: &S,
) -> Result<ApplicationBotAssets, BotAssetError> {
    let mut mounts = content.map_entities_content();
    let mut native = content.load_bot_files(&mounts);
    if simulation.has_q3_source_or_guest() {
        return Ok(ApplicationBotAssets::Q3 { files: native });
    }
    let family_is_q1 = simulation.has_q1_source();
    if native.read("bots/weapons.txt").is_none() {
        let expectation = if family_is_q1 {
            "q1-rerelease-id1"
        } else {
            "q2-rerelease-baseq2"
        };
        if let Some(sibling) = content.installed_sibling_content(expectation) {
            mounts.clone_from(&sibling);
            native = content.load_bot_files(&mounts);
        }
    }
    if native.read("bots/weapons.txt").is_some() {
        let source = if family_is_q1 {
            RereleaseBotSource::Q1
        } else {
            RereleaseBotSource::Q2
        };
        let mut knowledge = match source {
            RereleaseBotSource::Q1 => load_quake1_knowledge(&native)?.ok_or(BotAssetError::Disappeared)?,
            RereleaseBotSource::Q2 => bot_load_knowledge(&native, SettingsPlatform::Pc)?,
        };
        if !knowledge.errors.is_empty() {
            return Err(BotAssetError::Invalid(knowledge.errors.join("; ")));
        }
        let selected = content.selected_weapon();
        if selected
            .as_ref()
            .is_some_and(|selected| selected.content != content.map_entities_content())
        {
            let selected = selected.expect("selected weapon checked");
            let mut arsenal = content.load_bot_files(&selected.content);
            if !selected.provider.starts_with("q3:") && arsenal.read("bots/weapons.txt").is_none() {
                let expectation = if selected.provider.starts_with("q1:") {
                    "q1-rerelease-id1"
                } else {
                    "q2-rerelease-baseq2"
                };
                if let Some(sibling) = content.installed_sibling_content(expectation) {
                    arsenal = content.load_bot_files(&sibling);
                }
            }
            let weapons = if selected.provider.starts_with("q3:") {
                native_q3_weapon_knowledge(&arsenal)?
            } else if selected.provider.starts_with("q1:") {
                load_quake1_knowledge(&arsenal)?
                    .map(|knowledge| knowledge.weapons)
                    .unwrap_or_default()
            } else {
                bot_load_knowledge(&arsenal, SettingsPlatform::Pc)?.weapons
            };
            if weapons.is_empty() {
                return Err(BotAssetError::LacksArsenal);
            }
            knowledge.replace_weapons(weapons);
            let provenance = format!(
                "{}\n{}\n{}",
                selected.provider,
                selected.content,
                arsenal.provenance().unwrap_or_else(|| "undefined".to_string())
            );
            native.add("arsenal/provenance", provenance.as_bytes());
        }
        let language = simulation.bot_language().unwrap_or_else(|| "english".to_string());
        let language = if language.is_empty() {
            "english".to_string()
        } else {
            language
        };
        let profile = match source {
            RereleaseBotSource::Q1 => LocalizationProfile::Q1Rerelease,
            RereleaseBotSource::Q2 => LocalizationProfile::Q2Rerelease,
        };
        let mut reader = MountReader {
            content,
            mounts,
            native: &mut native,
        };
        let localization = load_server_localization_resources(&language, &mut reader, profile);
        drop(reader);
        return Ok(ApplicationBotAssets::Rerelease {
            source,
            files: native,
            knowledge: Box::new(knowledge),
            localization,
        });
    }
    let q3 = content.q3_base_content();
    Ok(ApplicationBotAssets::Q3 {
        files: content.load_bot_files(&q3),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeContent {
        map: String,
        files: std::collections::HashMap<String, BotAssetFiles>,
    }

    impl BotAssetContent for FakeContent {
        fn map_entities_content(&self) -> String {
            self.map.clone()
        }
        fn selected_weapon(&self) -> Option<BotAssetWeapon> {
            None
        }
        fn load_bot_files(&self, content: &str) -> BotAssetFiles {
            self.files.get(content).cloned().unwrap_or_default()
        }
        fn installed_sibling_content(&self, _expectation: &str) -> Option<String> {
            None
        }
        fn q3_base_content(&self) -> String {
            "q3-base".to_string()
        }
        fn open_content_file(&self, _content: &str, _path: &str) -> Option<Vec<u8>> {
            None
        }
    }

    struct FakeSimulation {
        q3: bool,
        q1: bool,
    }

    impl BotAssetSimulation for FakeSimulation {
        fn has_q3_source_or_guest(&self) -> bool {
            self.q3
        }
        fn has_q1_source(&self) -> bool {
            self.q1
        }
        fn bot_language(&self) -> Option<String> {
            None
        }
    }

    #[test]
    fn q3_sources_keep_mounted_files() {
        let mut files = std::collections::HashMap::new();
        let mut native = BotAssetFiles::new();
        native.add("bots/default.c", b"// bots");
        files.insert("map".to_string(), native);
        let content = FakeContent {
            map: "map".to_string(),
            files,
        };
        let simulation = FakeSimulation { q3: true, q1: false };
        let assets = load_application_bot_assets(&content, &simulation).unwrap();
        assert!(matches!(assets, ApplicationBotAssets::Q3 { .. }));
    }

    #[test]
    fn classic_products_without_definitions_fall_back_to_q3_base() {
        let mut files = std::collections::HashMap::new();
        let mut base = BotAssetFiles::new();
        base.add("bots/default.c", b"// q3");
        files.insert("q3-base".to_string(), base);
        files.insert("map".to_string(), BotAssetFiles::new());
        let content = FakeContent {
            map: "map".to_string(),
            files,
        };
        let simulation = FakeSimulation { q3: false, q1: false };
        let assets = load_application_bot_assets(&content, &simulation).unwrap();
        match assets {
            ApplicationBotAssets::Q3 { files } => {
                assert!(files.read("bots/default.c").is_some());
            }
            ApplicationBotAssets::Rerelease { .. } => panic!("expected q3 fallback"),
        }
    }
}
