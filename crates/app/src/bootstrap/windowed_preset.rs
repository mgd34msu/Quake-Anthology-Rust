//! Live launch collaborators for windowed campaign presets.
//!
//! Donor provenance: `src/app/bootstrap/startup-selection.ts`
//! (`StartupSelectionModel.resolvePreset`, which calls `applicationPreset`
//! from `src/app/bootstrap/content.ts` and then `resolveLaunch`, with no
//! collaborator seam). The Rust selection model inverts that dependency
//! through [`StartupSelectionCollaborators`](super::startup_selection::StartupSelectionCollaborators),
//! and the windowed smoke composition in [`super::windowed`] answers the
//! preset call with an honest error (`no launch preset in windowed smoke`),
//! so selecting a campaign Play item can never leave the menu.
//!
//! [`WindowedPresetCollaborators`] is the live composition for that seam:
//! player products and the launch preset come from the ported
//! [`super::content`] functions, and weapon sources come from the ported
//! [`CatalogWeaponSources`](qa_content::catalog::weapons::CatalogWeaponSources).
//! The remaining seam methods keep their honest windowed folds: the menu
//! composition has no mod browser, no arena progression store, and no
//! Team Arena mount reads, and campaign presets resolve TypeScript
//! execution (never QVM), so the QVM compatibility read stays a loud
//! error instead of invented behavior.

use qa_content::catalog::weapons::CatalogWeaponSources;
use qa_content::catalog::BehaviorMounts;
use qa_content::catalog::CatalogError;
use qa_content::catalog::InstalledCatalog;
use qa_content::catalog::LaunchPreset;
use qa_content::catalog::LaunchQvmCompatibility;
use qa_content::catalog::LaunchWeaponSources;
use qa_content::catalog::QvmCompatRole;
use qa_content::contract::ContentDigest;
use qa_content::contract::ExecutableRecipe;
use qa_content::contract::GameFamily;
use qa_content::contract::ModDescription;
use qa_content::contract::ModSelection;
use qa_content::contract::ProviderReference;
use qa_content::contract::QvmAbiProfile;
use qa_content::mounts::MountPreparationScope;

use super::content::application_player_products;
use super::content::application_preset;
use super::content::NativePresentationSources;
use super::startup_selection::PreparedQ3Catalog;
use super::startup_selection::PreparedTeamArena;
use super::startup_selection::QvmGrappleStyle;
use super::startup_selection::StartupArenaSelection;
use super::startup_selection::StartupPlayerProducts;
use super::startup_selection::StartupSelectionCollaborators;
use crate::options::ApplicationOptions;
use crate::options::GameFamily as OptionsFamily;
use crate::options::Network;

/// Live [`StartupSelectionCollaborators`] for windowed campaign launches.
pub struct WindowedPresetCollaborators;

/// QVM compatibility for windowed preset launches (never invoked: campaign
/// presets resolve TypeScript execution, and the read stays loud so a
/// future QVM preset fails honestly instead of guessing an ABI profile).
struct WindowedPresetQvmCompat;

impl LaunchQvmCompatibility for WindowedPresetQvmCompat {
    fn read_qvm_compatibility(
        &self,
        _mounts: &dyn BehaviorMounts,
        _artifact_path: &str,
        _digest: &ContentDigest,
        _role: QvmCompatRole,
    ) -> Result<QvmAbiProfile, CatalogError> {
        Err(CatalogError::Invalid("no qvm in windowed preset launches".to_string()))
    }
}

/// Contract family to options family (both enumerate Q1/Q2/Q3).
fn options_family(family: GameFamily) -> OptionsFamily {
    match family {
        GameFamily::Q1 => OptionsFamily::Q1,
        GameFamily::Q2 => OptionsFamily::Q2,
        GameFamily::Q3 => OptionsFamily::Q3,
    }
}

impl StartupSelectionCollaborators for WindowedPresetCollaborators {
    fn duplicate(&self) -> Box<dyn StartupSelectionCollaborators> {
        Box::new(WindowedPresetCollaborators)
    }

    fn mod_choices(&self, _catalog: &InstalledCatalog) -> Result<Vec<ModDescription>, String> {
        Ok(Vec::new())
    }

    fn apply_mods(
        &self,
        recipe: ExecutableRecipe,
        _choices: &[ModDescription],
        _mods: &[ModSelection],
    ) -> Result<ExecutableRecipe, String> {
        Ok(recipe)
    }

    fn read_arena_selection(
        &self,
        _catalog: &InstalledCatalog,
        _options: &ApplicationOptions,
    ) -> Result<StartupArenaSelection, String> {
        Ok(StartupArenaSelection::default())
    }

    fn prepare_q3_product(
        &self,
        catalog: &InstalledCatalog,
        _product_id: &str,
        _initial: &ApplicationOptions,
    ) -> Result<PreparedQ3Catalog, String> {
        Ok(PreparedQ3Catalog {
            catalog: catalog.clone(),
            q3_product: None,
        })
    }

    fn load_team_arena(
        &self,
        _catalog: &InstalledCatalog,
        _initial: &ApplicationOptions,
    ) -> Result<Option<PreparedTeamArena>, String> {
        Ok(None)
    }

    fn qvm_grapple_selection(
        &self,
        _catalog: &InstalledCatalog,
        _product_id: &str,
        _mounts: &MountPreparationScope,
    ) -> Result<Option<QvmGrappleStyle>, String> {
        Ok(None)
    }

    fn player_products(
        &self,
        catalog: &InstalledCatalog,
        product: &str,
        movement: GameFamily,
        character: GameFamily,
        network: &Network,
    ) -> Result<StartupPlayerProducts, String> {
        let options = ApplicationOptions {
            product: product.to_string(),
            movement: options_family(movement),
            character: options_family(character),
            network: network.clone(),
            ..ApplicationOptions::default()
        };
        let products = application_player_products(catalog, &options, None).map_err(|error| error.to_string())?;
        Ok(StartupPlayerProducts {
            movement: products.movement,
            character: products.character,
        })
    }

    fn application_preset(
        &self,
        catalog: &InstalledCatalog,
        options: &ApplicationOptions,
        movement: Option<&ProviderReference>,
        character: Option<&ProviderReference>,
    ) -> Result<LaunchPreset, String> {
        let sources;
        let native = match (movement, character) {
            (Some(movement), Some(character)) => {
                sources = NativePresentationSources {
                    movement: movement.clone(),
                    character: character.clone(),
                };
                Some(&sources)
            }
            _ => None,
        };
        application_preset(catalog, options, native, None).map_err(|error| error.to_string())
    }

    fn launch_weapons(&self) -> Box<dyn LaunchWeaponSources> {
        Box::new(CatalogWeaponSources)
    }

    fn launch_compat(&self) -> Box<dyn LaunchQvmCompatibility> {
        Box::new(WindowedPresetQvmCompat)
    }
}

#[cfg(test)]
mod tests {
    use qa_content::catalog::CatalogArchive;
    use qa_content::catalog::CatalogArchiveEntry;
    use qa_content::catalog::CatalogProduct;
    use qa_content::catalog::ContentMap;
    use qa_content::catalog::ProductAvailability;
    use qa_content::catalog::ProductExpectation;
    use qa_content::contract::ArchiveFormat;
    use qa_content::contract::ContentId;

    use super::super::startup_selection::StartupSelectionModel;
    use super::*;

    /// Minimal PAK bytes: the launch resolver opens the mount plan and
    /// resolves the campaign map, so the test archive must be a real
    /// readable file (contents are only digested, never parsed).
    fn build_pak(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut payloads = Vec::new();
        let mut dir = Vec::new();
        let mut offset = 12u32;
        for (name, bytes) in entries {
            let mut raw = [0u8; 56];
            raw[..name.len()].copy_from_slice(name.as_bytes());
            dir.extend_from_slice(&raw);
            dir.extend_from_slice(&offset.to_le_bytes());
            dir.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            payloads.extend_from_slice(bytes);
            offset += bytes.len() as u32;
        }
        let mut data = Vec::new();
        data.extend_from_slice(b"PACK");
        data.extend_from_slice(&offset.to_le_bytes());
        data.extend_from_slice(&(dir.len() as u32).to_le_bytes());
        data.extend_from_slice(&payloads);
        data.extend_from_slice(&dir);
        data
    }

    fn catalog() -> InstalledCatalog {
        let path = std::env::temp_dir().join(format!("qa-wu19-preset-{}.pak", std::process::id()));
        let bytes = build_pak(&[("maps/base1.bsp", &[7u8; 16]), ("players/male/tris.md2", &[9u8; 8])]);
        std::fs::write(&path, bytes).unwrap();
        let archive = path.to_string_lossy().into_owned();
        InstalledCatalog::new(
            "windowed-preset-test".to_string(),
            vec![CatalogProduct {
                id: ContentId("q2-classic-baseq2".to_string()),
                expectation: ProductExpectation {
                    id: "q2-classic-baseq2".to_string(),
                    family: GameFamily::Q2,
                    edition: "classic".to_string(),
                    campaign: "baseq2".to_string(),
                    title: "Quake II".to_string(),
                    content_directory: "baseq2".to_string(),
                    base_product: None,
                    required_content_archives: Vec::new(),
                    required_programs: Vec::new(),
                    map_witness: Some("maps/base1.bsp".to_string()),
                    unresolved_reason: None,
                },
                availability: ProductAvailability::Installed,
                archives: vec![CatalogArchive {
                    path: archive.clone(),
                    format: ArchiveFormat::Pak,
                    entries: vec![
                        CatalogArchiveEntry {
                            path: "maps/base1.bsp".to_string(),
                            ordinal: 0,
                            byte_length: 16,
                        },
                        CatalogArchiveEntry {
                            path: "players/male/tris.md2".to_string(),
                            ordinal: 1,
                            byte_length: 8,
                        },
                    ],
                }],
                loose_root: None,
                user_content: None,
                maps: vec![ContentMap {
                    path: "maps/base1.bsp".to_string(),
                    source: archive,
                    member_index: Some(0),
                }],
                diagnostics: Vec::new(),
            }],
            Vec::new(),
            0,
            None,
        )
        .unwrap()
    }

    fn model() -> StartupSelectionModel {
        let mut model = StartupSelectionModel::new(
            catalog(),
            ApplicationOptions::default(),
            Box::new(WindowedPresetCollaborators),
        )
        .unwrap();
        model.prepare_maps().unwrap();
        model
    }

    #[test]
    fn player_products_follow_live_resolution() {
        let collaborators = WindowedPresetCollaborators;
        let products = collaborators
            .player_products(
                &catalog(),
                "q2-classic-baseq2",
                GameFamily::Q2,
                GameFamily::Q2,
                &Network::Offline,
            )
            .unwrap();
        assert_eq!(products.movement, "q2-classic-baseq2");
        assert_eq!(products.character, "q2-classic-baseq2");
    }

    #[test]
    fn campaign_preset_resolves_to_launch_options() {
        let mut owned = model();
        let launch = owned.resolve_preset("q2-classic-baseq2", Some(1), None).unwrap();
        assert_eq!(launch.options.product, "q2-classic-baseq2");
        assert_eq!(launch.options.map, "maps/base1.bsp");
        assert_eq!(launch.options.character_model, "male");
        assert_eq!(launch.options.skill, 1);
        assert_eq!(launch.recipe.execution.len(), 1);
    }

    #[test]
    fn unknown_preset_still_fails() {
        let mut owned = model();
        let error = owned.resolve_preset("q9-elsewhere", None, None).unwrap_err();
        assert!(error.to_string().contains("q9-elsewhere"), "{error:?}");
    }
}
