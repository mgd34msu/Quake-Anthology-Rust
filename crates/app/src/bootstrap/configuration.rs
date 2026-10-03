//! Profile configuration entry points (port of Quake-Anthology-TS
//! `src/app/bootstrap/configuration.ts`).
//!
//! This port covers the donor's self-contained exports:
//! [`ConfigurationCommandRequest`], [`configuration_dialect`],
//! [`legacy_configuration_options`], [`configuration_store`], and
//! [`open_initial_configuration_content`].
//!
//! Scope note: `PreparedProfileConfiguration`, `prepareProfileConfiguration`,
//! and `prepareInitialConfiguration` are NOT ported here. Both functions are
//! orchestrators over sibling lanes that now live here
//! ([`prepared_startup`](super::prepared_startup),
//! [`startup_config`](super::startup_config),
//! [`startup_source`](super::startup_source) including
//! [`resolve_startup_rules`](super::startup_source::resolve_startup_rules),
//! [`player_userinfo`](super::player_userinfo),
//! [`gtv_commands`](super::gtv_commands),
//! [`input_devices`](super::input_devices),
//! [`q1_client_commands`](super::q1_client_commands),
//! [`q2_client_commands`](super::q2_client_commands),
//! [`q3_map_command`](super::q3_map_command),
//! [`team_arena_skirmish`](super::team_arena_skirmish),
//! [`image_settings`](super::image_settings),
//! [`view_settings`](super::view_settings),
//! [`audio_settings`](super::audio_settings); only the `MouseSettings` class
//! is unported ([`qa_client::input::mouse_settings`] carries the tuning
//! functions)). Porting the orchestrators now would mean inventing their
//! composition API as a speculative seam, which the stay-in-scope rule
//! forbids. They belong to the integration lane.

use super::config_scripts::LegacyConsoleConfigSources;
use super::content::{
    application_configuration_preset, application_discovers_mods, open_application_configuration_content,
    ApplicationConfigurationContent, ApplicationConfigurationRequest, ApplicationContentOptions, Q3ProductPreparer,
};
use crate::options::ApplicationOptions;
use crate::settings::config::ConfigStore;
use qa_client::ui::types::CommandDialect;
use qa_content::catalog::{discover_installed_content, DiscoverContentOptions, InstalledCatalog};
use qa_content::contract::{ContentId, ExecutableRecipe, GameFamily};
use qa_core::cmd_buffer::CommandContext;
use qa_core::identity::SeatId;
use std::path::PathBuf;

/// Configuration failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigurationError {
    /// Donor error with its exact message.
    Message(String),
}

impl std::fmt::Display for ConfigurationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for ConfigurationError {}

impl From<qa_content::catalog::CatalogError> for ConfigurationError {
    fn from(error: qa_content::catalog::CatalogError) -> Self {
        Self::Message(error.to_string())
    }
}

impl From<super::content::ContentError> for ConfigurationError {
    fn from(error: super::content::ContentError) -> Self {
        Self::Message(error.to_string())
    }
}

impl From<qa_content::paths::PathError> for ConfigurationError {
    fn from(error: qa_content::paths::PathError) -> Self {
        Self::Message(error.to_string())
    }
}

/// Application command request target (donor `"application"` literal).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConfigurationCommandTarget {
    /// The host application.
    Application,
}

/// Deferred application command (donor `ConfigurationCommandRequest`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigurationCommandRequest {
    /// Request target.
    pub target: ConfigurationCommandTarget,
    /// Command name.
    pub name: String,
    /// Command arguments.
    pub arguments: Vec<String>,
    /// Originating seat, when local.
    pub seat: Option<SeatId>,
    /// Originating context.
    pub source: CommandContext,
}

/// Open configuration content for launch or a recipe (donor
/// `openInitialConfigurationContent`). Q3 product preparation reuses the
/// content lane's [`Q3ProductPreparer`] seam (donor `q3-product.ts`).
pub fn open_initial_configuration_content<Q>(
    options: &ApplicationOptions,
    recipe: Option<ExecutableRecipe>,
    installed_catalog: Option<InstalledCatalog>,
    q3: &mut Q,
) -> Result<ApplicationConfigurationContent, ConfigurationError>
where
    Q: Q3ProductPreparer,
    Q::Error: Into<super::content::ContentError>,
{
    let mut catalog = match installed_catalog {
        Some(catalog) => catalog,
        None => {
            let user_content_root = options
                .user_content_root
                .as_ref()
                .map(PathBuf::from)
                .or_else(|| Some(qa_content::user_data::default_user_content_root()));
            discover_installed_content(&DiscoverContentOptions {
                corpus_root: PathBuf::from(&options.corpus_root),
                user_content_root,
                products: None,
                generation: 0,
                discover_mods: application_discovers_mods(options, recipe.as_ref()),
                remote_content: None,
            })?
        }
    };
    let product = recipe.as_ref().map_or_else(
        || options.product.clone(),
        |recipe| recipe.map.entities.content.as_str().to_string(),
    );
    let content_options = ApplicationContentOptions {
        base: options.clone(),
        remote_content: None,
        q3_product: None,
    };
    let prepared = q3
        .prepare_q3_application_product(catalog, &product, &content_options)
        .map_err(Into::into)
        .map_err(ConfigurationError::from)?;
    catalog = prepared.catalog;
    let q3_selected = prepared.q3_product;
    if let Some(recipe) = recipe {
        return open_application_configuration_content(
            catalog,
            ApplicationConfigurationRequest::Recipe { recipe },
            q3_selected,
            q3,
        )
        .map_err(Into::into);
    }
    let preset = application_configuration_preset(&catalog, options, None)?;
    let choice = qa_content::catalog::preset_choice(preset.id.clone());
    open_application_configuration_content(
        catalog,
        ApplicationConfigurationRequest::Launch { preset, choice },
        q3_selected,
        q3,
    )
    .map_err(Into::into)
}

/// Command dialect for configuration content (donor `configurationDialect`).
pub fn configuration_dialect(content: &ApplicationConfigurationContent) -> Result<CommandDialect, ConfigurationError> {
    let source = &content
        .catalog
        .product(content.selection.engine_behavior.content.as_str())?
        .expectation;
    Ok(match source.family {
        GameFamily::Q1 if source.edition == "quakeworld" => CommandDialect::Q1Quakeworld,
        GameFamily::Q1 => CommandDialect::Q1Netquake,
        GameFamily::Q2 if source.edition == "rerelease" => CommandDialect::Q2Rerelease,
        GameFamily::Q2 => CommandDialect::Q2Classic,
        GameFamily::Q3 => CommandDialect::Q3,
    })
}

/// Legacy console configuration roots (donor `legacyConfigurationOptions`).
pub fn legacy_configuration_options(
    options: &ApplicationOptions,
    catalog: &InstalledCatalog,
    reference: &ContentId,
) -> Result<Option<LegacyConsoleConfigSources>, ConfigurationError> {
    let selected = catalog.product(reference.as_str())?;
    if selected.expectation.family != GameFamily::Q1 || selected.expectation.edition == "quakeworld" {
        return Ok(None);
    }
    let shared_root = options
        .user_content_root
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(qa_content::user_data::default_user_content_root);
    let mut game_roots = Vec::new();
    for product in
        std::iter::once(selected).chain(catalog.products.iter().filter(|product| {
            product.expectation.family == GameFamily::Q1 && product.expectation.edition != "quakeworld"
        }))
    {
        let root = match &product.user_content {
            Some(user) => PathBuf::from(&user.root),
            None => {
                qa_content::user_data::user_product_directory(&shared_root, &product.expectation.content_directory)?
            }
        };
        if !game_roots.contains(&root) {
            game_roots.push(root);
        }
    }
    Ok(Some(LegacyConsoleConfigSources {
        shared_root,
        game_roots,
    }))
}

/// Configuration store for a content product (donor `configurationStore`).
pub fn configuration_store(
    options: &ApplicationOptions,
    content: &ApplicationConfigurationContent,
    reference: &ContentId,
) -> Result<ConfigStore, ConfigurationError> {
    let product = content.catalog.product(reference.as_str())?;
    let root = match &product.user_content {
        Some(user) => PathBuf::from(&user.root),
        None => {
            let shared = options
                .user_content_root
                .as_ref()
                .map(PathBuf::from)
                .unwrap_or_else(qa_content::user_data::default_user_content_root);
            qa_content::user_data::user_product_directory(&shared, &product.expectation.content_directory)?
        }
    };
    Ok(ConfigStore::new(root))
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::catalog::{CatalogProduct, ProductAvailability, ProductExpectation};
    use qa_content::contract::{ContentMount, LooseMount, MountPlanId, ResolvedMountPlan};
    use qa_content::mounts::{open_mount_plan, OpenMountOptions};
    use qa_core::cmd::Dialect;
    use qa_core::cmd_buffer::CommandOrigin;
    use qa_core::identity::IdentityOwner;

    fn expectation(id: &str, family: GameFamily, edition: &str) -> ProductExpectation {
        ProductExpectation {
            id: id.to_string(),
            family,
            edition: edition.to_string(),
            campaign: "test".to_string(),
            title: id.to_string(),
            content_directory: format!("dir-{id}"),
            base_product: None,
            required_content_archives: Vec::new(),
            required_programs: Vec::new(),
            map_witness: None,
            unresolved_reason: None,
        }
    }

    fn product(id: &str, family: GameFamily, edition: &str) -> CatalogProduct {
        CatalogProduct {
            id: ContentId(id.to_string()),
            expectation: expectation(id, family, edition),
            availability: ProductAvailability::Installed,
            archives: Vec::new(),
            loose_root: None,
            user_content: None,
            maps: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn catalog() -> InstalledCatalog {
        InstalledCatalog::new(
            "corpus".to_string(),
            vec![
                product("q1-classic-id1", GameFamily::Q1, "classic"),
                product("q1-qw-id1", GameFamily::Q1, "quakeworld"),
                product("q2-classic-baseq2", GameFamily::Q2, "classic"),
                product("q2-rerelease-baseq2", GameFamily::Q2, "rerelease"),
                product("q3-retail-baseq3", GameFamily::Q3, "retail"),
            ],
            Vec::new(),
            0,
            None,
        )
        .unwrap()
    }

    fn mounts() -> qa_content::mounts::MountedContent {
        let content = ContentId("configuration-test".to_string());
        let mount = LooseMount {
            identity: qa_content::contract::create_mount_identity(
                qa_content::contract::create_mount_id("configuration", "test").unwrap(),
                content,
                0,
            )
            .unwrap(),
            root_path: std::env::temp_dir()
                .join("qa-configuration-mounts")
                .to_string_lossy()
                .into_owned(),
        };
        open_mount_plan(
            &ResolvedMountPlan {
                id: MountPlanId("mount-plan:configuration:test".to_string()),
                mounts: vec![ContentMount::Loose(mount.clone())],
                default_order: vec![mount.identity.id.clone()],
                prefix_orders: Vec::new(),
            },
            OpenMountOptions::default(),
        )
        .unwrap()
    }

    fn selection(content: &str) -> super::super::content::ApplicationConfigurationSelection {
        use qa_content::contract::ProviderReference;
        use qa_core::identity::ProviderId;
        let provider = ProviderReference {
            provider: ProviderId::new("provider", "test:engine"),
            content: ContentId(content.to_string()),
        };
        super::super::content::ApplicationConfigurationSelection {
            source: provider.clone(),
            engine_behavior: provider.clone(),
            r#match: provider.clone(),
            combat: provider.clone(),
            movement: provider,
            timing: Vec::new(),
        }
    }

    fn content_for(engine: &str) -> ApplicationConfigurationContent {
        ApplicationConfigurationContent {
            q3_product: None,
            catalog: catalog(),
            selection: selection(engine),
            mounts: mounts(),
        }
    }

    fn options() -> ApplicationOptions {
        ApplicationOptions {
            product: "q1-classic-id1".to_string(),
            map: "start".to_string(),
            ..ApplicationOptions::default()
        }
    }

    #[test]
    fn configuration_dialect_maps_family_and_edition() {
        assert_eq!(
            configuration_dialect(&content_for("q1-classic-id1")).unwrap(),
            CommandDialect::Q1Netquake
        );
        assert_eq!(
            configuration_dialect(&content_for("q1-qw-id1")).unwrap(),
            CommandDialect::Q1Quakeworld
        );
        assert_eq!(
            configuration_dialect(&content_for("q2-classic-baseq2")).unwrap(),
            CommandDialect::Q2Classic
        );
        assert_eq!(
            configuration_dialect(&content_for("q2-rerelease-baseq2")).unwrap(),
            CommandDialect::Q2Rerelease
        );
        assert_eq!(
            configuration_dialect(&content_for("q3-retail-baseq3")).unwrap(),
            CommandDialect::Q3
        );
        assert!(configuration_dialect(&content_for("missing")).is_err());
    }

    #[test]
    fn legacy_configuration_options_cover_netquake_only() {
        let catalog = catalog();
        let options = options();
        let sources = legacy_configuration_options(&options, &catalog, &ContentId("q1-classic-id1".to_string()))
            .unwrap()
            .unwrap();
        assert_eq!(sources.game_roots.len(), 1);
        assert!(sources.game_roots[0].ends_with("dir-q1-classic-id1"));
        assert!(
            legacy_configuration_options(&options, &catalog, &ContentId("q1-qw-id1".to_string()))
                .unwrap()
                .is_none()
        );
        assert!(
            legacy_configuration_options(&options, &catalog, &ContentId("q2-classic-baseq2".to_string()))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn configuration_store_roots_at_product_directory() {
        let content = content_for("q1-classic-id1");
        let store = configuration_store(&options(), &content, &ContentId("q1-classic-id1".to_string())).unwrap();
        assert!(store.root.ends_with("dir-q1-classic-id1"));
    }

    #[test]
    fn open_initial_reports_missing_products() {
        struct StubQ3;
        impl Q3ProductPreparer for StubQ3 {
            type Error = super::super::content::ContentError;
            fn prepare_q3_application_product(
                &mut self,
                catalog: InstalledCatalog,
                _product: &str,
                _options: &ApplicationContentOptions,
            ) -> Result<super::super::content::PreparedQ3Product, Self::Error> {
                Ok(super::super::content::PreparedQ3Product {
                    catalog,
                    q3_product: None,
                })
            }
        }
        let mut options = options();
        options.product = "missing-product".to_string();
        let error = open_initial_configuration_content(&options, None, Some(catalog()), &mut StubQ3).unwrap_err();
        assert!(matches!(error, ConfigurationError::Message(_)));
    }

    #[test]
    fn configuration_command_request_carries_target() {
        let owner = IdentityOwner::create("configuration-test").unwrap();
        let seat = owner.seat(0);
        let request = ConfigurationCommandRequest {
            target: ConfigurationCommandTarget::Application,
            name: "map".to_string(),
            arguments: vec!["q3dm1".to_string()],
            seat: Some(seat.clone()),
            source: CommandContext::new(owner.session().clone(), CommandOrigin::LocalConsole),
        };
        assert_eq!(request.target, ConfigurationCommandTarget::Application);
        assert_eq!(request.seat, Some(seat));
        let _ = Dialect::Q3;
    }
}
