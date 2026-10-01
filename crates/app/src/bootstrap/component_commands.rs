//! Component console commands staying explicit when original components emit text.
//!
//! Port of donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/component-commands.ts`
//! (`componentEngineCommands`). Source-administration names arrive through a
//! caller-provided lookup because the server-administration lane is unported;
//! the `sv` filter, Q3 map commands, and fixed engine names stay here.

use std::collections::HashSet;

use qa_client::ui::types::CommandDialect;

use super::audio::commands::APPLICATION_AUDIO_COMMANDS;
use super::content::{q3_product_map_commands, Q3ProductPolicy};
use super::content::{ApplicationContentPreparer, LoadedApplicationContent};

/// Engine command names a component may emit (`componentEngineCommands`).
pub fn component_engine_commands<P>(
    dialect: CommandDialect,
    content: &LoadedApplicationContent<'_, P>,
    source_administration_names: &dyn Fn(CommandDialect) -> Vec<String>,
) -> HashSet<String>
where
    P: ApplicationContentPreparer,
{
    let policy = content
        .q3_product
        .map(|product| product.policy)
        .unwrap_or(Q3ProductPolicy::Retail);
    source_administration_names(dialect)
        .into_iter()
        .filter(|name| name != "sv")
        .chain(APPLICATION_AUDIO_COMMANDS.into_iter().map(str::to_string))
        .chain(q3_product_map_commands(policy).iter().map(|name| (*name).to_string()))
        .chain(
            [
                "quit",
                "map",
                "gamemap",
                "changelevel",
                "map_restart",
                "save",
                "load",
                "exec",
                "cinematic",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .map(|name| name.to_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::content::{
        load_application_content, ApplicationContentOptions, ConfiguredApplicationRecipe, ContentError, ContentScope,
        ModPurpose, Q2Edition, WorldByteArena,
    };
    use super::*;
    use qa_content::catalog::{
        expected_products, BehaviorMounts, InstalledCatalog, LaunchQvmCompatibility, ProductAvailability, QvmCompatRole,
    };
    use qa_content::contract::{
        CampaignSelection, CharacterSelection, ContentDigest, ContentId, DopplerSelection, EnemySelection,
        EnvironmentSelection, ExecutableRecipe, ExecutionModule, FrameOrdering, GameFamily, GrappleSelection,
        ModuleRole, PresentationSelection, ProviderReference, QuakeCApiIdentity, RecipeId, ResolvedExecutionModule,
        ResolvedMap, ResolvedMountPlan, ResolvedWeaponBehaviorSelection, SourceModuleApi,
    };
    use qa_content::mounts::{MountedContent, OpenMountOptions};
    use qa_core::identity::ProviderId;
    use std::fs;

    struct StubPreparer;

    impl ApplicationContentPreparer for StubPreparer {
        type Error = ContentError;
        type Mod = ();
        type WeaponBehavior = ();
        type QvmGrapple = ();
        type QuakeC = ();
        type Q3Game = ();
        type Q2Game = ();

        fn prepare_configured_recipe(
            &mut self,
            catalog: InstalledCatalog,
            _options: &ApplicationContentOptions,
            recipe: ExecutableRecipe,
        ) -> Result<ConfiguredApplicationRecipe, ContentError> {
            Ok(ConfiguredApplicationRecipe { catalog, recipe })
        }

        fn prepare_mods(
            &mut self,
            _catalog: &InstalledCatalog,
            _recipe: &ExecutableRecipe,
            _scope: &mut dyn ContentScope,
            _purpose: ModPurpose,
        ) -> Result<Vec<()>, ContentError> {
            Ok(Vec::new())
        }

        fn prepare_weapon_behavior(
            &mut self,
            _catalog: &InstalledCatalog,
            _selection: &ResolvedWeaponBehaviorSelection,
            _scope: &mut dyn ContentScope,
        ) -> Result<(), ContentError> {
            Ok(())
        }

        fn prepare_qvm_grapple(
            &mut self,
            _selection: &GrappleSelection,
            _scope: &mut dyn ContentScope,
        ) -> Result<(), ContentError> {
            Ok(())
        }

        fn prepare_quakec_source(
            &mut self,
            _execution: &ResolvedExecutionModule,
            _mounts: &MountedContent,
            _entities: &str,
            _source: Option<&MountedContent>,
        ) -> Result<(), ContentError> {
            Ok(())
        }

        fn quakec_weapon_stage_qualified(&self, _prepared: &()) -> bool {
            true
        }

        fn quakec_damage_scaling_qualified(&self, _prepared: &()) -> bool {
            true
        }

        fn prepare_q3_game(
            &mut self,
            _execution: &ResolvedExecutionModule,
            _mounts: &MountedContent,
        ) -> Result<(), ContentError> {
            Ok(())
        }

        fn q3_primary_complete(&self, _prepared: &()) -> bool {
            true
        }

        fn prepare_q2_guest(
            &mut self,
            _execution: &ResolvedExecutionModule,
            _mounts: &MountedContent,
            _edition: Q2Edition,
        ) -> Result<(), ContentError> {
            Ok(())
        }

        fn native_primary_present(&self, _prepared: &()) -> bool {
            true
        }

        fn prepare_native_q2_map(
            &mut self,
            _world: &super::super::content::ApplicationWorld<'_>,
            _edition: Q2Edition,
            _mode: crate::options::GameMode,
        ) -> Result<(), ContentError> {
            Ok(())
        }
    }

    struct StubQ3 {
        product: Option<super::super::content::Q3ApplicationProduct>,
    }

    impl super::super::content::Q3ProductPreparer for StubQ3 {
        type Error = ContentError;

        fn prepare_q3_application_product(
            &mut self,
            catalog: InstalledCatalog,
            _product: &str,
            _options: &ApplicationContentOptions,
        ) -> Result<super::super::content::PreparedQ3Product, ContentError> {
            Ok(super::super::content::PreparedQ3Product {
                catalog,
                q3_product: self.product,
            })
        }
    }

    struct StubCompat;

    impl LaunchQvmCompatibility for StubCompat {
        fn read_qvm_compatibility(
            &self,
            _mounts: &dyn BehaviorMounts,
            _artifact_path: &str,
            _digest: &ContentDigest,
            _role: QvmCompatRole,
        ) -> Result<qa_content::contract::QvmAbiProfile, qa_content::catalog::CatalogError> {
            Ok(qa_content::contract::QvmAbiProfile::Modern)
        }
    }

    fn q1_test_bsp() -> Vec<u8> {
        let mut bytes = vec![0u8; 4 + 15 * 8];
        bytes[0] = 29;
        bytes
    }

    fn loaded_with_q3(
        tag: &str,
        product: Option<super::super::content::Q3ApplicationProduct>,
    ) -> LoadedApplicationContent<'static, StubPreparer> {
        let root: std::path::PathBuf =
            std::env::temp_dir().join(format!("qa-component-commands-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let maps = root.join("maps");
        fs::create_dir_all(&maps).unwrap();
        fs::write(maps.join("test.bsp"), q1_test_bsp()).unwrap();
        let products = expected_products()
            .into_iter()
            .map(|expectation| {
                let loose_root = if expectation.id == "q1-classic-id1" {
                    Some(root.display().to_string())
                } else {
                    None
                };
                qa_content::catalog::CatalogProduct {
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
        let catalog = InstalledCatalog::new(root.display().to_string(), products, Vec::new(), 0, None).unwrap();
        let mounts = catalog.mounts_for("q1-classic-id1").unwrap();
        let plan = ResolvedMountPlan {
            id: qa_content::contract::create_mount_plan_id("test", "commands").unwrap(),
            default_order: mounts.iter().map(|mount| mount.identity().id.clone()).collect(),
            mounts,
            prefix_orders: Vec::new(),
        };
        let probe = qa_content::mounts::open_mount_plan(&plan, OpenMountOptions::default()).unwrap();
        let geometry = probe
            .open("maps/test.bsp", |_| true)
            .unwrap()
            .unwrap()
            .reference
            .clone();
        probe.close();
        let source = ProviderReference {
            provider: ProviderId::new("q1", "official"),
            content: ContentId("q1-classic-id1".to_string()),
        };
        let recipe = ExecutableRecipe {
            weapon_behaviors: Vec::new(),
            mods: Vec::new(),
            schema_version: 3,
            id: RecipeId("recipe:test:commands".to_string()),
            preset: RecipeId("recipe:test:preset".to_string()),
            map: ResolvedMap {
                geometry_content: ContentId("q1-classic-id1".to_string()),
                geometry: geometry.clone(),
                entities: source.clone(),
            },
            campaign: CampaignSelection::None,
            movement: ProviderReference {
                provider: ProviderId::new("q1", "movement"),
                content: ContentId("q1-classic-id1".to_string()),
            },
            character: CharacterSelection {
                definition: ProviderReference {
                    provider: ProviderId::new("q1", "character"),
                    content: ContentId("q1-classic-id1".to_string()),
                },
                appearance: ProviderReference {
                    provider: ProviderId::new("q1", "model/ranger"),
                    content: ContentId("q1-classic-id1".to_string()),
                },
            },
            weapons: vec![source.clone()],
            equipment: qa_content::catalog::native_equipment(&catalog, &source, &source).unwrap(),
            enemies: EnemySelection::MapDefined,
            presentation: PresentationSelection {
                doppler: DopplerSelection::Source,
                environment: EnvironmentSelection::AudioContent,
                assets: ContentId("q1-classic-id1".to_string()),
                hud: source.clone(),
                effects: source.clone(),
                audio: source.clone(),
            },
            engine_behavior: source.clone(),
            combat: source.clone(),
            inventory: source.clone(),
            r#match: source.clone(),
            transition: source.clone(),
            execution: vec![ExecutionModule::Typescript {
                owner: source.clone(),
                implementation: ProviderId::new("q1", "official"),
                role: ModuleRole::ServerGame,
                api: SourceModuleApi::Quakec(QuakeCApiIdentity::Netquake),
            }],
            mounts: plan,
            resources: vec![geometry],
            timing: vec![qa_content::catalog::native_provider_timing(
                &source,
                GameFamily::Q1,
                false,
            )],
            ordering: FrameOrdering::Mixed {
                providers: vec![source.provider.clone()],
            },
        };
        let options = ApplicationContentOptions::new(crate::options::ApplicationOptions {
            product: "q1-classic-id1".to_string(),
            map: "maps/test.bsp".to_string(),
            ..crate::options::ApplicationOptions::default()
        });
        let arena = Box::leak(Box::new(WorldByteArena::default()));
        let loaded = load_application_content(
            &options,
            Some(recipe),
            None,
            Some(catalog),
            None,
            &StubCompat,
            &mut StubQ3 { product },
            &mut StubPreparer,
            arena,
        )
        .unwrap();
        // The arena intentionally outlives the test so the world stays borrowed.
        loaded
    }

    #[test]
    fn engine_commands_cover_retail_q3_maps() {
        let loaded = loaded_with_q3("retail", None);
        let names = component_engine_commands(CommandDialect::Q3, &loaded, &|_| {
            vec!["sv".to_string(), "Status".to_string()]
        });
        assert!(!names.contains("sv"));
        assert!(names.contains("status"));
        assert!(names.contains("snd_restart"));
        assert!(names.contains("devmap"));
        assert!(names.contains("quit"));
        assert!(names.contains("cinematic"));
    }

    #[test]
    fn engine_commands_follow_demo_policy() {
        use super::super::content::{Q3ApplicationProduct, Q3MountRestriction, Q3ProductPolicy};
        let loaded = loaded_with_q3(
            "demo",
            Some(Q3ApplicationProduct {
                policy: Q3ProductPolicy::PrereleaseDemo {
                    team_arena_ui: super::super::content::Q3TeamArenaUi::Retail,
                },
                restriction: Q3MountRestriction::None,
            }),
        );
        let names = component_engine_commands(CommandDialect::Q3, &loaded, &|_| Vec::new());
        assert!(names.contains("map"));
        assert!(!names.contains("devmap"));
    }
}
