//! Single-player arena selection rows and saved-selection reads.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/base-arena-selection.ts`
//! (`arenaSelection`, `readArenaSelection`). The initial configuration content
//! comes from [`ArenaSelectionConfiguration`], the narrow seam over
//! [`configuration`](super::configuration); tier labels, record text, and
//! current-arena fallback stay here.

use qa_content::catalog::InstalledCatalog;
use qa_content::contract::ProviderReference;
use qa_content::mounts::MountedContent;
use qa_core::cmd::Dialect;
use qa_core::cvar::{flags, CvarRegistry, SetCommandKind};
use thiserror::Error;

use super::base_arena_catalog::{read_base_arena_catalog, BaseArena, BaseArenaCatalog};
use super::base_arena_progression::{ArenaProgressionCatalog, BaseArenaProgression, BaseArenaProgressionError};
use super::cvar_archives::{load_cvar_archive, CvarArchiveError, CvarArchiveHead, CvarArchiveOwner};
use crate::options::ApplicationOptions;
use crate::settings::config::ConfigStore;

/// Arena selection failure.
#[derive(Debug, Error)]
pub enum ArenaSelectionError {
    /// Catalog failure.
    #[error(transparent)]
    Catalog(#[from] super::base_arena_catalog::BaseArenaCatalogError),
    /// Progression failure.
    #[error(transparent)]
    Progression(#[from] BaseArenaProgressionError),
    /// Cvar failure.
    #[error(transparent)]
    Cvar(#[from] qa_core::cvar::CvarError),
    /// Archive failure.
    #[error(transparent)]
    Archive(#[from] CvarArchiveError),
}

/// One selectable arena row (`ArenaSelectionRow`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArenaSelectionRow {
    /// Arena.
    pub arena: BaseArena,
    /// Tier id.
    pub tier: String,
    /// Whether the level is unlocked.
    pub available: bool,
    /// Best-rank record text.
    pub record: String,
}

/// Tier label (`ArenaSelection` tiers entry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArenaSelectionTier {
    /// Tier id.
    pub id: String,
    /// Tier label.
    pub label: String,
}

/// Arena selection (`ArenaSelection`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArenaSelection {
    /// Tiers in first-appearance order.
    pub tiers: Vec<ArenaSelectionTier>,
    /// Rows in selection order.
    pub rows: Vec<ArenaSelectionRow>,
    /// Current map, if any arena is available.
    pub current: Option<String>,
}

fn tier_label(tier: &str) -> String {
    match tier {
        "training" => "Training".to_string(),
        "final" => "Final arena".to_string(),
        tier => format!("Tier {tier}"),
    }
}

/// Build selection rows from a catalog and progression (`arenaSelection`).
pub fn arena_selection(
    catalog: &BaseArenaCatalog,
    progress: &BaseArenaProgression,
    saved_selection: &str,
) -> Result<ArenaSelection, ArenaSelectionError> {
    let mut arenas: Vec<&BaseArena> = catalog
        .arenas
        .iter()
        .filter(|arena| !arena.special.is_empty() || arena.number < catalog.regular_count)
        .collect();
    arenas.sort_by_key(|arena| arena.selection);
    let mut tiers: Vec<ArenaSelectionTier> = Vec::new();
    let mut rows: Vec<ArenaSelectionRow> = Vec::new();
    for arena in arenas {
        let special = arena.special.to_lowercase();
        let tier = if special == "training" {
            "training".to_string()
        } else if special == "final" {
            "final".to_string()
        } else {
            (arena.number.div_euclid(4) + 1).to_string()
        };
        if !tiers.iter().any(|entry| entry.id == tier) {
            tiers.push(ArenaSelectionTier {
                id: tier.clone(),
                label: tier_label(&tier),
            });
        }
        let best = progress.best(arena.number)?;
        rows.push(ArenaSelectionRow {
            arena: arena.clone(),
            tier,
            available: progress.level_available(arena.number)?,
            record: if best.rank == 0 {
                "Not completed".to_string()
            } else {
                format!("Rank {} · Skill {}", best.rank, best.skill)
            },
        });
    }
    let saved = if saved_selection.trim().is_empty() {
        None
    } else {
        saved_selection
            .trim()
            .parse::<i32>()
            .ok()
            .and_then(|selection| {
                rows.iter()
                    .find(|row| row.available && row.arena.selection == selection)
            })
            .map(|row| row.arena.map.clone())
    };
    let current_level = progress.current_level()?;
    let current = saved
        .or_else(|| {
            catalog
                .arenas
                .iter()
                .find(|arena| arena.number == current_level)
                .map(|arena| arena.map.clone())
        })
        .or_else(|| rows.iter().find(|row| row.available).map(|row| row.arena.map.clone()));
    Ok(ArenaSelection { tiers, rows, current })
}

/// Initial configuration content for selection reads.
pub struct InitialArenaSelectionConfiguration {
    /// Configuration mounts.
    pub mounts: MountedContent,
    /// Source provider.
    pub source: ProviderReference,
    /// Configuration store.
    pub store: ConfigStore,
}

impl InitialArenaSelectionConfiguration {
    /// Close the configuration mounts.
    pub fn close(self) {
        self.mounts.close();
    }
}

/// Initial configuration content (donor `configuration.ts`).
pub trait ArenaSelectionConfiguration {
    /// Open failure.
    type Error;
    /// Open the initial configuration content without starting a world.
    fn open_initial_configuration(
        &mut self,
        options: &ApplicationOptions,
        installed: &InstalledCatalog,
    ) -> Result<InitialArenaSelectionConfiguration, Self::Error>;
}

/// Read the selection from the source profile (`readArenaSelection`).
pub fn read_arena_selection<C>(
    installed: &InstalledCatalog,
    options: &ApplicationOptions,
    configuration: &mut C,
) -> Result<ArenaSelection, ArenaSelectionError>
where
    C: ArenaSelectionConfiguration,
    C::Error: Into<ArenaSelectionError>,
{
    let content = configuration
        .open_initial_configuration(options, installed)
        .map_err(Into::into)?;
    let result = read_arena_selection_inner(&content);
    content.close();
    result
}

fn read_arena_selection_inner(
    content: &InitialArenaSelectionConfiguration,
) -> Result<ArenaSelection, ArenaSelectionError> {
    let catalog = read_base_arena_catalog(&content.mounts)?;
    let cvars = CvarRegistry::new(Dialect::Q3);
    let mut progress = BaseArenaProgression::new(
        cvars,
        ArenaProgressionCatalog {
            regular_levels: catalog.regular_count,
            training: catalog
                .arenas
                .iter()
                .find(|arena| arena.special.to_lowercase() == "training")
                .map(|arena| arena.number),
            final_level: catalog
                .arenas
                .iter()
                .find(|arena| arena.special.to_lowercase() == "final")
                .map(|arena| arena.number),
            total_levels: catalog.regular_count
                + catalog.arenas.iter().filter(|arena| !arena.special.is_empty()).count() as i32,
        },
    )?;
    progress.cvars_mut().register("ui_spSelection", "", flags::ARCHIVE)?;
    let source = &content.source;
    let owner = CvarArchiveOwner::new(
        CvarArchiveHead::Source,
        vec![source.content.as_str().to_string(), provider_text(&source.provider)],
    );
    let entries = load_cvar_archive(&content.store, &owner, Dialect::Q3)?;
    let archived: Vec<(&str, &str)> = entries
        .iter()
        .filter(|entry| progress.cvars().get(&entry.name).is_some())
        .map(|entry| (entry.name.as_str(), entry.value.as_str()))
        .collect();
    for (name, value) in archived {
        progress
            .cvars_mut()
            .set_command_flags(name, value, SetCommandKind::Archive)?;
    }
    let saved = progress.cvars().variable_string("ui_spSelection");
    arena_selection(&catalog, &progress, &saved)
}

fn provider_text(provider: &qa_core::identity::ProviderId) -> String {
    format!("{}:{}", provider.namespace, provider.name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::catalog::{expected_products, CatalogProduct, ProductAvailability};
    use qa_content::contract::ContentId;
    use qa_core::identity::ProviderId;

    fn test_arena(number: i32, map: &str, special: &str, selection: i32) -> BaseArena {
        BaseArena {
            number,
            map: map.to_string(),
            title: map.to_string(),
            bots: Vec::new(),
            special: special.to_string(),
            selection,
            frag_limit: 0,
            time_limit: 0,
        }
    }

    #[test]
    fn selection_tiers_rows_and_fallback() {
        let catalog = BaseArenaCatalog {
            arenas: vec![
                test_arena(0, "maps/training.bsp", "training", 0),
                test_arena(1, "maps/q3dm1.bsp", "", 1),
                test_arena(2, "maps/q3dm2.bsp", "", 2),
            ],
            regular_count: 4,
            tier_count: 1,
        };
        let progress = BaseArenaProgression::new(
            CvarRegistry::new(Dialect::Q3),
            ArenaProgressionCatalog {
                regular_levels: 4,
                training: Some(0),
                final_level: None,
                total_levels: 5,
            },
        )
        .unwrap();
        let mut progress = progress;
        let selection = arena_selection(&catalog, &progress, "").unwrap();
        assert_eq!(selection.tiers.len(), 2);
        assert_eq!(selection.tiers[0].label, "Training");
        assert_eq!(selection.tiers[1].label, "Tier 1");
        assert_eq!(selection.rows.len(), 3);
        assert_eq!(selection.rows[0].record, "Not completed");
        assert!(selection.rows[0].available);
        assert_eq!(selection.current.as_deref(), Some("maps/training.bsp"));
        progress
            .record(&super::super::base_arena_progression::ArenaResult {
                level: 0,
                skill: super::super::base_arena_progression::ArenaSkill::One,
                rank: 1,
                accuracy: 50,
                impressive: 0,
                excellent: 0,
                gauntlet: 0,
                frags: 10,
                perfect: false,
            })
            .unwrap();
        let fallback = arena_selection(&catalog, &progress, "").unwrap();
        assert_eq!(fallback.current.as_deref(), Some("maps/q3dm1.bsp"));
        let saved = arena_selection(&catalog, &progress, "0").unwrap();
        assert_eq!(saved.current.as_deref(), Some("maps/training.bsp"));
    }

    #[test]
    fn selection_ignores_blank_and_unknown_saved() {
        let catalog = BaseArenaCatalog {
            arenas: vec![test_arena(0, "maps/q3dm1.bsp", "", 0)],
            regular_count: 4,
            tier_count: 1,
        };
        let progress = BaseArenaProgression::new(
            CvarRegistry::new(Dialect::Q3),
            ArenaProgressionCatalog {
                regular_levels: 4,
                training: None,
                final_level: None,
                total_levels: 4,
            },
        )
        .unwrap();
        let blank = arena_selection(&catalog, &progress, "   ").unwrap();
        assert_eq!(blank.current.as_deref(), Some("maps/q3dm1.bsp"));
        let unknown = arena_selection(&catalog, &progress, "nope").unwrap();
        assert_eq!(unknown.current.as_deref(), Some("maps/q3dm1.bsp"));
    }

    #[test]
    fn read_selection_uses_saved_and_store() {
        struct StubConfiguration {
            mounts: Option<MountedContent>,
            store: Option<ConfigStore>,
        }

        impl ArenaSelectionConfiguration for StubConfiguration {
            type Error = ArenaSelectionError;

            fn open_initial_configuration(
                &mut self,
                _options: &ApplicationOptions,
                _installed: &InstalledCatalog,
            ) -> Result<InitialArenaSelectionConfiguration, ArenaSelectionError> {
                Ok(InitialArenaSelectionConfiguration {
                    mounts: self.mounts.take().unwrap(),
                    source: ProviderReference {
                        provider: ProviderId::new("q3", "official"),
                        content: ContentId("q3-baseq3".to_string()),
                    },
                    store: self.store.take().unwrap(),
                })
            }
        }

        let root: std::path::PathBuf = std::env::temp_dir().join(format!("qa-arena-selection-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let scripts = root.join("scripts");
        std::fs::create_dir_all(&scripts).unwrap();
        let mut arenas = String::new();
        for index in 1..=4 {
            arenas.push_str(&format!(
                "{{\n  map \"q3dm{index}\"\n  longname \"Arena {index}\"\n  type \"single\"\n  bots \"sarge major\"\n  fraglimit \"10\"\n  timelimit \"0\"\n}}\n"
            ));
        }
        std::fs::write(scripts.join("arenas.txt"), arenas).unwrap();
        let products = expected_products()
            .into_iter()
            .map(|expectation| {
                let loose_root = if expectation.id == "q3-baseq3" {
                    Some(root.display().to_string())
                } else {
                    None
                };
                CatalogProduct {
                    id: ContentId(expectation.id.clone()),
                    expectation,
                    availability: ProductAvailability::Installed,
                    archives: Vec::new(),
                    loose_root,
                    user_content: None,
                    maps: Vec::new(),
                    diagnostics: Vec::new(),
                }
            })
            .collect();
        let installed = InstalledCatalog::new(root.display().to_string(), products, Vec::new(), 0, None).unwrap();
        let mounts = installed.mounts_for("q3-baseq3").unwrap();
        let plan = qa_content::contract::ResolvedMountPlan {
            id: qa_content::contract::create_mount_plan_id("test", "arena").unwrap(),
            default_order: mounts.iter().map(|mount| mount.identity().id.clone()).collect(),
            mounts,
            prefix_orders: Vec::new(),
        };
        let mounted =
            qa_content::mounts::open_mount_plan(&plan, qa_content::mounts::OpenMountOptions::default()).unwrap();
        let store = ConfigStore::new(root.join("config"));
        let mut configuration = StubConfiguration {
            mounts: Some(mounted),
            store: Some(store),
        };
        let selection = read_arena_selection(&installed, &ApplicationOptions::default(), &mut configuration).unwrap();
        assert_eq!(selection.rows.len(), 4);
        assert_eq!(selection.current.as_deref(), Some("maps/q3dm1.bsp"));
    }
}
