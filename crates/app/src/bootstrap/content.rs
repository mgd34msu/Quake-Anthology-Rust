//! Application content selection and loading.
//!
//! Donor: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/content.ts`
//! (`openRemoteContent`, `openRemoteApplicationContent`,
//! `remoteConfigurationContent`, `applicationSourceSelection`,
//! `applicationDiscoversMods`, `resolveApplicationMovement`,
//! `applicationPlayerProducts`, `applicationPreset`,
//! `applicationConfigurationPreset`, `openApplicationConfigurationContent`,
//! `LoadedApplicationContent`, `applicationOptionsForRecipe`,
//! `resolveApplicationTravel`, `loadApplicationContent`).
//! Async mounts/catalog access becomes sync calls. Q3 product policy is
//! absorbed from `src/core/q3-product-policy.ts` (`Q3ApplicationProduct`,
//! `q3ProductMapCommands`); the options lane has not ported the donor's
//! `remoteContent`/`q3Product` option fields, so they travel in
//! [`ApplicationContentOptions`]. Unported collaborators (Q3 product
//! preparation, QVM compatibility, mods/weapons/grapple selection, guest
//! sources) arrive through [`Q3ProductPreparer`], [`LaunchQvmCompatibility`],
//! and [`ApplicationContentPreparer`]. BSP bytes live in a caller-provided
//! arena (`map_bytes`) because the decoded worlds borrow them. The `.ent`
//! and `.lit` sidecars retain their archive identity in
//! [`ApplicationMapSidecar`] and thread into [`read_q1_bsp`] through
//! [`Q1BspOptions`], mirroring the donor's `readQ1Bsp` options.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use qa_content::bsp::{read_q1_bsp, Q1BspOptions, Q1Map};
use qa_content::bsp2::{read_q2_bsp, to_q2_world_geometry, Q2DecodedMap, Q2MapResources};
use qa_content::bsp3::{decode_q3_world, Q3DecodedWorld};
use qa_content::catalog::weapons::CatalogWeaponSources;
use qa_content::catalog::{
    discover_installed_content, expected_products, native_equipment, native_provider_timing, prepare_launch_mount_plan,
    preset_choice, remote_content_product, remote_content_selection, resolve_launch, resolve_launch_resource,
    source_program_implementation, source_program_product, CatalogProduct, DiscoverContentOptions, InstalledCatalog,
    LaunchPreset, LaunchQvmCompatibility, LaunchResourceKind, RemoteContentBase, RemoteContentSelection,
    ResolveLaunchOptions,
};
use qa_content::contract::{
    create_mount_plan_id, create_recipe_id, CampaignSelection, CharacterSelection, ContentId, ContentMount,
    DopplerSelection, EnemySelection, EnvironmentSelection, ExecutableRecipe, ExecutionModule, FrameOrdering,
    GameFamily, GrappleMechanicDetail, GrappleSelection, LaunchChoice, MapSelection, ModuleRole, NativeAbi,
    NativeModuleApi, PresentationSelection, ProviderReference, ProviderTiming, Q3ApiIdentity, QuakeCApiIdentity,
    ResolvedExecutionModule, ResolvedMap, ResolvedMountPlan, ResolvedWeaponBehaviorSelection, ResourceRequest,
    SourceModuleApi,
};
use qa_content::hash::hex_lower;
use qa_content::mounts::{
    open_mount_plan, MountError, MountedContent, OpenMountOptions, PureMountPolicy, Q3Restriction, ResourceRef,
};
use qa_content::paths::{find_content_path, PathComparison};
use qa_content::user_data::{default_user_content_root, user_product_directory};
use qa_content::{classify_bsp, BspKind};
use qa_core::binary::BinaryError;
use qa_core::identity::ProviderId;
use qa_core::time::ClockProfile;
use thiserror::Error;

use super::demo_playback::DemoFamily;
use crate::options::{ApplicationOptions, GameFamily as OptionsFamily, GameMode, MatchRules, Network};

/// Application content failure.
#[derive(Debug, Error)]
pub enum ContentError {
    /// Remote base product is unknown.
    #[error("Missing remote base product {0}")]
    MissingRemoteBase(String),
    /// Remote download directory does not match its content owner.
    #[error("Remote download directory does not match its content owner")]
    RemoteDirectoryMismatch,
    /// Q3 guest recipe requirements not met.
    #[error("Q3 bytecode requires one native Q3 server game, native movement and character, map-defined actors and supported source providers without campaign")]
    Q3GuestRecipe,
    /// Remote content context requires its matching remote client product.
    #[error("Remote content context requires its matching remote client product")]
    RemoteProductMismatch,
    /// Recorded content family differs from the selected product.
    #[error("Recorded content family differs from the selected product")]
    RecordedFamilyMismatch,
    /// Rules require a classic Quake II game provider.
    #[error("{0} requires a classic Quake II game provider")]
    RulesNeedClassicQ2(String),
    /// Tag and DeathBall require Ground Zero or Quake II rerelease.
    #[error("Tag and DeathBall require Ground Zero or Quake II rerelease")]
    RulesNeedRogue,
    /// Horde requires Quake rerelease MG1 or DOPA.
    #[error("Horde requires Quake rerelease MG1 or DOPA")]
    RulesNeedHorde,
    /// Selected Q3 mods require an offline local or dedicated server.
    #[error("Selected Q3 mods require an offline local or dedicated server with native Q3 movement and character and deathmatch")]
    Q3GuestRejected,
    /// Native QuakeWorld requirements not met.
    #[error("Native QuakeWorld requires dedicated deathmatch with Q1 character presentation and its native protocol")]
    QuakeWorldRejected,
    /// Q2 game library requirements not met.
    #[error("--q2-game requires Quake II source, movement and character with native offline/server operation")]
    Q2GameRejected,
    /// QuakeC program requirements not met.
    #[error("--progs requires an offline/server NetQuake source world; native wire servers require Q1 character presentation")]
    QuakeCRejected,
    /// Q3 bytecode requirements not met.
    #[error("Q3 bytecode requires offline local or dedicated Q3 server operation")]
    QvmRejected,
    /// QuakeC requirements not met.
    #[error("QuakeC requires matching source actors and validated artifact; native network maps must belong to the source content and QuakeWorld requires dedicated operation")]
    QuakeCModuleRejected,
    /// Native Quake II requirements not met.
    #[error("Native Quake II requires edition-matching actors, movement, character and arsenal")]
    NativeQ2Rejected,
    /// Unsupported execution module.
    #[error("{0}")]
    UnsupportedModule(String),
    /// Native Quake clients require Quake BSP geometry.
    #[error("Native Quake clients require Quake BSP geometry")]
    NativeClientGeometry,
    /// Selected arsenal requires a qualified QuakeC weapon stage.
    #[error("Selected arsenal requires a qualified original QuakeC weapon stage")]
    QuakeCWeaponStage,
    /// Selected arsenal requires a qualified QuakeC damage scale.
    #[error("Selected arsenal requires a qualified original QuakeC damage scale")]
    QuakeCDamageScale,
    /// Selected QVM arsenal requires complete interfaces.
    #[error("Selected QVM arsenal requires complete artifact-qualified primary player, weapon, inventory, pickup and combat interfaces")]
    QvmArsenal,
    /// Selected native arsenal requires a complete primary declaration.
    #[error("Selected native arsenal requires a complete original primary declaration")]
    NativeArsenal,
    /// Native or bytecode game has no shared projectile behavior hook.
    #[error("Selected native or bytecode game has no shared projectile behavior hook")]
    WeaponBehaviorHook,
    /// Conflicting trajectory behaviors for one projectile role.
    #[error("Conflicting trajectory behaviors for one projectile role")]
    WeaponBehaviorConflict,
    /// No server-approved archives provide the content.
    #[error("No server-approved archives provide {0}")]
    NoPureArchives(String),
    /// Application content is closed.
    #[error("Application content is closed")]
    Closed,
    /// Application content closed during mount.
    #[error("Application content closed during mount")]
    ClosedDuringMount,
    /// Application input has no adapter for the provider.
    #[error("Application input has no adapter for {0}")]
    NoInputAdapter(String),
    /// Application character has no model selection.
    #[error("Application character has no model selection for {0}")]
    NoCharacterModel(String),
    /// Filesystem failure.
    #[error("Application content IO failed for {path}: {message}")]
    Io {
        /// Requested path.
        path: String,
        /// Underlying error.
        message: String,
    },
    /// Catalog failure.
    #[error(transparent)]
    Catalog(#[from] qa_content::catalog::CatalogError),
    /// Mount failure.
    #[error(transparent)]
    Mount(#[from] MountError),
    /// Contract failure.
    #[error(transparent)]
    Contract(#[from] qa_content::contract::ContractError),
    /// Map decode failure.
    #[error(transparent)]
    Map(#[from] BinaryError),
    /// Path failure.
    #[error(transparent)]
    Path(#[from] qa_content::paths::PathError),
}

/// Q3 product policy (`Q3ProductPolicy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q3ProductPolicy {
    /// Retail.
    Retail,
    /// Prerelease demo.
    PrereleaseDemo {
        /// Team Arena UI policy.
        team_arena_ui: Q3TeamArenaUi,
    },
    /// Prerelease Team Arena demo.
    PrereleaseTaDemo,
}

/// Prerelease Team Arena UI policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q3TeamArenaUi {
    /// Retail UI.
    Retail,
    /// Demo UI.
    Demo,
}

/// Q3 mount restriction (`Q3MountRestriction`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q3MountRestriction {
    /// No restriction.
    None,
    /// Demo restriction.
    Demo {
        /// Demo directory (`demota`).
        directory: &'static str,
        /// Demo pak checksum.
        pak_checksum: u32,
    },
}

/// Q3 application product (`Q3ApplicationProduct`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Q3ApplicationProduct {
    /// Product policy.
    pub policy: Q3ProductPolicy,
    /// Mount restriction.
    pub restriction: Q3MountRestriction,
}

/// Build a Q3 product policy (`q3ProductPolicy`).
#[must_use]
pub fn q3_product_policy(prerelease_demo: bool, prerelease_team_arena_demo: bool) -> Q3ProductPolicy {
    if prerelease_demo {
        Q3ProductPolicy::PrereleaseDemo {
            team_arena_ui: if prerelease_team_arena_demo {
                Q3TeamArenaUi::Demo
            } else {
                Q3TeamArenaUi::Retail
            },
        }
    } else if prerelease_team_arena_demo {
        Q3ProductPolicy::PrereleaseTaDemo
    } else {
        Q3ProductPolicy::Retail
    }
}

/// Whether the policy is a prerelease demo (`q3PrereleaseDemo`).
#[must_use]
pub fn q3_prerelease_demo(policy: Q3ProductPolicy) -> bool {
    matches!(policy, Q3ProductPolicy::PrereleaseDemo { .. })
}

/// Whether the policy is a Team Arena demo (`q3TeamArenaDemo`).
#[must_use]
pub fn q3_team_arena_demo(policy: Q3ProductPolicy) -> bool {
    matches!(policy, Q3ProductPolicy::PrereleaseTaDemo)
        || matches!(
            policy,
            Q3ProductPolicy::PrereleaseDemo {
                team_arena_ui: Q3TeamArenaUi::Demo
            }
        )
}

/// Mount restriction for a policy (`q3MountRestriction`).
#[must_use]
pub fn q3_mount_restriction(policy: Q3ProductPolicy, fs_restrict: bool) -> Q3MountRestriction {
    if fs_restrict || q3_prerelease_demo(policy) {
        Q3MountRestriction::Demo {
            directory: "demota",
            pak_checksum: 437_558_517,
        }
    } else {
        Q3MountRestriction::None
    }
}

/// Map commands for a Q3 product policy (`q3ProductMapCommands`).
#[must_use]
pub fn q3_product_map_commands(policy: Q3ProductPolicy) -> &'static [&'static str] {
    if q3_prerelease_demo(policy) {
        &["map"]
    } else {
        &["map", "devmap", "spmap", "spdevmap"]
    }
}

/// Recording source kind for presentation-only loads
/// (`ApplicationContentSource`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApplicationContentSource {
    /// Recorded or unified source.
    pub kind: ApplicationContentSourceKind,
    /// Recording family.
    pub family: DemoFamily,
}

/// Recording source kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApplicationContentSourceKind {
    /// Recorded demo.
    Recorded,
    /// Unified recipe.
    Unified,
}

/// Decoded application world (`ApplicationWorld`).
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum ApplicationWorld<'a> {
    /// Quake map.
    Q1(Q1Map<'a>),
    /// Quake II world geometry.
    Q2(Q2DecodedMap<'a>),
    /// Quake III world.
    Q3(Q3DecodedWorld),
}

impl ApplicationWorld<'_> {
    /// World kind marker (`world.kind`).
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Q1(_) => "q1-bsp",
            Self::Q2(_) => "q2-bsp",
            Self::Q3(_) => "q3-bsp",
        }
    }

    /// Entity text.
    #[must_use]
    pub fn entities(&self) -> &str {
        match self {
            Self::Q1(map) => &map.entities,
            Self::Q2(map) => &map.map.entities,
            Self::Q3(world) => &world.map.entities,
        }
    }
}

/// Map sidecar with its retained archive identity
/// (`ApplicationMapSidecar`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationMapSidecar {
    /// Sidecar content.
    pub content: ContentId,
    /// Sidecar path.
    pub path: String,
    /// Resolved reference, when the sidecar exists.
    pub resource: Option<qa_content::contract::ResolvedResourceReference>,
}

/// Donor `ApplicationOptions` fields the options lane has not ported yet,
/// paired with the ported base options.
#[derive(Debug, Clone)]
pub struct ApplicationContentOptions {
    /// Ported base options.
    pub base: ApplicationOptions,
    /// Remote content selection (`remoteContent`).
    pub remote_content: Option<RemoteContentSelection>,
    /// Q3 application product (`q3Product`).
    pub q3_product: Option<Q3ApplicationProduct>,
}

impl ApplicationContentOptions {
    /// Wrap base options without extras.
    #[must_use]
    pub fn new(base: ApplicationOptions) -> Self {
        Self {
            base,
            remote_content: None,
            q3_product: None,
        }
    }
}

/// Prepared Q3 application product (`prepareQ3ApplicationProduct` result).
#[derive(Debug, Clone)]
pub struct PreparedQ3Product {
    /// Catalog with the Q3 product prepared.
    pub catalog: InstalledCatalog,
    /// Q3 application product, when Q3 content is selected.
    pub q3_product: Option<Q3ApplicationProduct>,
}

/// Q3 application product preparation (donor `q3-product.ts`).
pub trait Q3ProductPreparer {
    /// Preparation failure.
    type Error;
    /// Prepare the Q3 application product for a catalog and product.
    fn prepare_q3_application_product(
        &mut self,
        catalog: InstalledCatalog,
        product: &str,
        options: &ApplicationContentOptions,
    ) -> Result<PreparedQ3Product, Self::Error>;
}

fn provider_text(id: &ProviderId) -> String {
    format!("{}:{}", id.namespace, id.name)
}

fn match_rules_text(rules: MatchRules) -> &'static str {
    match rules {
        MatchRules::Standard => "standard",
        MatchRules::Ctf => "ctf",
        MatchRules::Lmctf => "lmctf",
        MatchRules::Tag => "tag",
        MatchRules::Deathball => "deathball",
        MatchRules::Horde => "horde",
    }
}

fn network_is_offline(network: &Network) -> bool {
    matches!(network, Network::Offline)
}

fn network_is_native_server(network: &Network) -> bool {
    matches!(network, Network::NativeServer { .. })
}

fn network_is_q2_server(network: &Network) -> bool {
    matches!(network, Network::Q2Server { .. })
}

fn network_is_qw_client(network: &Network) -> bool {
    matches!(network, Network::QwClient { .. })
}

fn network_is_q3_client(network: &Network) -> bool {
    matches!(network, Network::Q3Client { .. })
}

fn base_product(family: GameFamily) -> &'static str {
    match family {
        GameFamily::Q1 => "q1-classic-id1",
        GameFamily::Q2 => "q2-classic-baseq2",
        GameFamily::Q3 => "q3-baseq3",
    }
}

fn content_family(family: OptionsFamily) -> GameFamily {
    match family {
        OptionsFamily::Q1 => GameFamily::Q1,
        OptionsFamily::Q2 => GameFamily::Q2,
        OptionsFamily::Q3 => GameFamily::Q3,
    }
}

fn options_family(family: GameFamily) -> OptionsFamily {
    match family {
        GameFamily::Q1 => OptionsFamily::Q1,
        GameFamily::Q2 => OptionsFamily::Q2,
        GameFamily::Q3 => OptionsFamily::Q3,
    }
}

fn typescript_execution(
    provider: &ProviderReference,
    family: GameFamily,
    rerelease: bool,
    implementation: ProviderId,
) -> ExecutionModule<ResourceRequest> {
    let api = match family {
        GameFamily::Q1 => SourceModuleApi::Quakec(QuakeCApiIdentity::Netquake),
        GameFamily::Q2 if rerelease => SourceModuleApi::Q2RereleaseGame,
        GameFamily::Q2 => SourceModuleApi::Q2ClassicGame,
        GameFamily::Q3 => SourceModuleApi::Q3Qagame(8),
    };
    ExecutionModule::Typescript {
        owner: provider.clone(),
        implementation,
        role: ModuleRole::ServerGame,
        api,
    }
}

/// Application source selection (`ApplicationSourceSelection`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationSourceSelection {
    /// Source provider.
    pub source: ProviderReference,
    /// Match provider.
    pub r#match: ProviderReference,
    /// Match rules.
    pub rules: MatchRules,
}

/// Resolve the source and match providers plus validated rules
/// (`applicationSourceSelection`).
pub fn application_source_selection(
    catalog: &InstalledCatalog,
    product: &str,
    rules: Option<MatchRules>,
) -> Result<ApplicationSourceSelection, ContentError> {
    let found = catalog.require(product)?;
    let family = found.expectation.family;
    let source = ProviderReference {
        provider: ProviderId::new(&family.to_string(), "official"),
        content: found.id.clone(),
    };
    let program = source_program_product(catalog, found.id.as_str())?.expectation.clone();
    let rerelease = program.edition == "rerelease";
    let rules = rules.unwrap_or_else(|| {
        if family == GameFamily::Q2 && !rerelease && (program.campaign == "ctf" || program.campaign == "lmctf") {
            if program.campaign == "ctf" {
                MatchRules::Ctf
            } else {
                MatchRules::Lmctf
            }
        } else {
            MatchRules::Standard
        }
    });
    if (rules == MatchRules::Ctf || rules == MatchRules::Lmctf) && (family != GameFamily::Q2 || rerelease) {
        return Err(ContentError::RulesNeedClassicQ2(match_rules_text(rules).to_string()));
    }
    if (rules == MatchRules::Tag || rules == MatchRules::Deathball)
        && (family != GameFamily::Q2 || (!rerelease && program.campaign != "rogue"))
    {
        return Err(ContentError::RulesNeedRogue);
    }
    if rules == MatchRules::Horde
        && (family != GameFamily::Q1 || !rerelease || (program.campaign != "mg1" && program.campaign != "dopa"))
    {
        return Err(ContentError::RulesNeedHorde);
    }
    let r#match = if rules == MatchRules::Standard {
        source.clone()
    } else if rules == MatchRules::Ctf || rules == MatchRules::Lmctf {
        ProviderReference {
            provider: ProviderId::new("q2", match_rules_text(rules)),
            content: catalog
                .require(&format!("q2-classic-{}", match_rules_text(rules)))?
                .id
                .clone(),
        }
    } else {
        ProviderReference {
            provider: ProviderId::new(&family.to_string(), match_rules_text(rules)),
            content: found.id.clone(),
        }
    };
    Ok(ApplicationSourceSelection { source, r#match, rules })
}

/// Whether content discovery includes mods (`applicationDiscoversMods`).
#[must_use]
pub fn application_discovers_mods(options: &ApplicationOptions, recipe: Option<&ExecutableRecipe>) -> bool {
    let product = expected_products()
        .into_iter()
        .find(|product| product.id == options.product);
    options.weapon_behavior.is_some()
        || !options.mods.is_empty()
        || options.dedicated
        || (network_is_offline(&options.network)
            && (recipe.is_some()
                || product.is_none()
                || product.is_some_and(|product| product.family == GameFamily::Q3)))
}

fn selected_q2_game_library(catalog: &InstalledCatalog, options: &ApplicationOptions) -> Option<String> {
    if let Some(library) = &options.q2_game_library {
        return Some(library.clone());
    }
    let product = source_program_product(catalog, &options.product).ok()?;
    let library = if product.expectation.edition == "rerelease" {
        "game_x64.dll"
    } else {
        "gamex86.dll"
    };
    if product.expectation.family == GameFamily::Q2
        && !expected_products()
            .iter()
            .any(|builtin| builtin.id == product.expectation.id)
        && product
            .expectation
            .required_programs
            .iter()
            .any(|program| program == library)
    {
        Some(library.to_string())
    } else {
        None
    }
}

fn selected_quakec_program(catalog: &InstalledCatalog, options: &ApplicationOptions) -> Option<String> {
    if let Some(program) = &options.quake_c_program {
        return Some(program.clone());
    }
    let product = source_program_product(catalog, &options.product).ok()?;
    if product.expectation.family == GameFamily::Q1
        && product.expectation.edition != "quakeworld"
        && !expected_products()
            .iter()
            .any(|builtin| builtin.id == product.expectation.id)
        && product
            .expectation
            .required_programs
            .iter()
            .any(|program| program == "progs.dat")
    {
        Some("progs.dat".to_string())
    } else {
        None
    }
}

/// Resolve the movement product override (`resolveApplicationMovement`).
pub fn resolve_application_movement(
    catalog: &InstalledCatalog,
    options: &ApplicationOptions,
) -> Result<ApplicationOptions, ContentError> {
    let Some(movement_product) = &options.movement_product else {
        return Ok(options.clone());
    };
    let product = catalog.require(movement_product)?;
    Ok(ApplicationOptions {
        movement: options_family(product.expectation.family),
        movement_product: Some(product.expectation.id.clone()),
        ..options.clone()
    })
}

/// Movement and character product identities (`applicationPlayerProducts`
/// result).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationPlayerProducts {
    /// Movement product.
    pub movement: String,
    /// Character product.
    pub character: String,
}

/// Resolve movement and character products (`applicationPlayerProducts`).
pub fn application_player_products(
    catalog: &InstalledCatalog,
    options: &ApplicationOptions,
    native_source: Option<bool>,
) -> Result<ApplicationPlayerProducts, ContentError> {
    let product = catalog.product(&options.product)?;
    let family = product.expectation.family;
    let own_player = native_source.unwrap_or_else(|| {
        product.availability == qa_content::catalog::ProductAvailability::Installed
            && ((product.expectation.edition == "quakeworld" && !network_is_qw_client(&options.network))
                || (family == GameFamily::Q3
                    && !network_is_q3_client(&options.network)
                    && !expected_products().iter().any(|builtin| {
                        builtin.id
                            == source_program_product(catalog, product.id.as_str())
                                .map(|program| program.expectation.id.clone())
                                .unwrap_or_default()
                    })))
    });
    Ok(ApplicationPlayerProducts {
        movement: match &options.movement_product {
            Some(movement) => catalog.require(movement)?.expectation.id.clone(),
            None if own_player && content_family(options.movement) == family => product.expectation.id.clone(),
            None => base_product(content_family(options.movement)).to_string(),
        },
        character: if own_player && content_family(options.character) == family {
            product.expectation.id.clone()
        } else {
            base_product(content_family(options.character)).to_string()
        },
    })
}

/// Native presentation sources (`applicationPreset` native sources).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativePresentationSources {
    /// Movement provider.
    pub movement: ProviderReference,
    /// Character provider.
    pub character: ProviderReference,
}

/// Build the launch preset, validating native execution requirements
/// (`applicationPreset`).
pub fn application_preset(
    catalog: &InstalledCatalog,
    options: &ApplicationOptions,
    native_sources: Option<&NativePresentationSources>,
    presentation_source: Option<ApplicationContentSource>,
) -> Result<LaunchPreset, ContentError> {
    let options = resolve_application_movement(catalog, options)?;
    let product = catalog.require(&options.product)?;
    let family = product.expectation.family;
    let q3_guest = family == GameFamily::Q3
        && presentation_source.is_none_or(|source| source.family != DemoFamily::Q3)
        && !network_is_q3_client(&options.network)
        && !expected_products().iter().any(|builtin| {
            builtin.id
                == source_program_product(catalog, product.id.as_str())
                    .map(|program| program.expectation.id.clone())
                    .unwrap_or_default()
        });
    if q3_guest
        && ((!options.dedicated && !network_is_offline(&options.network))
            || (!network_is_native_server(&options.network) && !network_is_offline(&options.network))
            || options.mode != GameMode::Deathmatch
            || options.movement != OptionsFamily::Q3
            || options.character != OptionsFamily::Q3)
    {
        return Err(ContentError::Q3GuestRejected);
    }
    let quakeworld = product.expectation.edition == "quakeworld"
        && presentation_source.is_none_or(|source| source.family != DemoFamily::Qw)
        && !network_is_qw_client(&options.network);
    let native_program = selected_quakec_program(catalog, &options);
    if selected_q2_game_library(catalog, &options).is_some()
        && (family != GameFamily::Q2
            || options.movement != OptionsFamily::Q2
            || options.character != OptionsFamily::Q2
            || options.quake_c_program.is_some()
            || options.bot_skill.is_some()
            || (!network_is_offline(&options.network)
                && !network_is_native_server(&options.network)
                && !network_is_q2_server(&options.network)))
    {
        return Err(ContentError::Q2GameRejected);
    }
    if native_program.is_some()
        && (family != GameFamily::Q1
            || product.expectation.edition == "quakeworld"
            || (!network_is_offline(&options.network) && !network_is_native_server(&options.network))
            || (network_is_native_server(&options.network) && options.character != OptionsFamily::Q1))
    {
        return Err(ContentError::QuakeCRejected);
    }
    if quakeworld
        && (!options.dedicated
            || options.mode != GameMode::Deathmatch
            || options.character != OptionsFamily::Q1
            || options.q1_protocol.is_some()
            || (!network_is_offline(&options.network) && !network_is_native_server(&options.network)))
    {
        return Err(ContentError::QuakeWorldRejected);
    }
    selected_application_preset(catalog, &options, native_sources, quakeworld, q3_guest)
}

/// Build the configuration preset (`applicationConfigurationPreset`).
pub fn application_configuration_preset(
    catalog: &InstalledCatalog,
    options: &ApplicationOptions,
    native_sources: Option<&NativePresentationSources>,
) -> Result<LaunchPreset, ContentError> {
    let options = resolve_application_movement(catalog, options)?;
    let product = catalog.require(&options.product)?;
    let family = product.expectation.family;
    let q3_guest = family == GameFamily::Q3
        && !expected_products().iter().any(|builtin| {
            builtin.id
                == source_program_product(catalog, product.id.as_str())
                    .map(|program| program.expectation.id.clone())
                    .unwrap_or_default()
        });
    let quakeworld = product.expectation.edition == "quakeworld";
    selected_application_preset(catalog, &options, native_sources, quakeworld, q3_guest)
}

fn selected_application_preset(
    catalog: &InstalledCatalog,
    options: &ApplicationOptions,
    native_sources: Option<&NativePresentationSources>,
    quakeworld: bool,
    q3_guest: bool,
) -> Result<LaunchPreset, ContentError> {
    let product = catalog.require(&options.product)?;
    let family = product.expectation.family;
    let native_program = selected_quakec_program(catalog, options);
    let q2_game_library = selected_q2_game_library(catalog, options);
    let selection = application_source_selection(catalog, &options.product, options.rules)?;
    let program_product = source_program_product(catalog, product.id.as_str())?;
    let equipment_source = if native_program.is_none() && q2_game_library.is_none() && !q3_guest && !quakeworld {
        ProviderReference {
            provider: selection.source.provider.clone(),
            content: program_product.id.clone(),
        }
    } else {
        selection.source.clone()
    };
    let player_products = application_player_products(catalog, options, Some(quakeworld || q3_guest))?;
    let movement = native_sources.map_or_else(
        || {
            Ok::<ProviderReference, ContentError>(ProviderReference {
                provider: ProviderId::new(&content_family(options.movement).to_string(), "movement"),
                content: catalog.require(&player_products.movement)?.id.clone(),
            })
        },
        |sources| Ok(sources.movement.clone()),
    )?;
    let character = native_sources.map_or_else(
        || {
            Ok::<ProviderReference, ContentError>(ProviderReference {
                provider: ProviderId::new(&content_family(options.character).to_string(), "character"),
                content: catalog.require(&player_products.character)?.id.clone(),
            })
        },
        |sources| Ok(sources.character.clone()),
    )?;
    let appearance = ProviderReference {
        provider: ProviderId::new(
            &content_family(options.character).to_string(),
            &format!("model/{}", options.character_model),
        ),
        content: character.content.clone(),
    };
    let rerelease = product.expectation.edition == "rerelease";
    let timing = |reference: &ProviderReference,
                  source: GameFamily,
                  edition: bool,
                  movement_role: bool|
     -> Result<ProviderTiming, ContentError> {
        let native = native_provider_timing(reference, source, edition);
        if source == GameFamily::Q1 && catalog.product(reference.content.as_str())?.expectation.edition == "quakeworld"
        {
            Ok(ProviderTiming {
                clock: ClockProfile::Q1Quakeworld {
                    maximum_command_milliseconds: if movement_role
                        && (options.movement_product.is_some() || native_sources.is_some())
                    {
                        255.0
                    } else {
                        50.0
                    },
                },
                ..native
            })
        } else {
            Ok(native)
        }
    };
    let provider_timing = timing(&selection.source, family, rerelease, false)?;
    let movement_edition = catalog.product(movement.content.as_str())?.expectation.edition == "rerelease";
    let character_edition = catalog.product(character.content.as_str())?.expectation.edition == "rerelease";
    let rules_suffix = if selection.rules == MatchRules::Standard {
        String::new()
    } else {
        format!("-{}", match_rules_text(selection.rules))
    };
    let movement_name = options
        .movement_product
        .clone()
        .unwrap_or_else(|| content_family(options.movement).to_string());
    let id = create_recipe_id(
        "mixed",
        &format!(
            "{}-{movement_name}-{}-{}{rules_suffix}",
            options.product,
            content_family(options.character),
            options.character_model
        ),
    )?;
    let execution = if let Some(library) = &q2_game_library {
        vec![ExecutionModule::Native {
            owner: selection.source.clone(),
            artifact: ResourceRequest {
                content: product.id.clone(),
                path: library.clone(),
            },
            profile: if rerelease {
                NativeAbi::WindowsX86_64
            } else {
                NativeAbi::WindowsI386
            },
            role: ModuleRole::ServerGame,
            api: if rerelease {
                NativeModuleApi::Q2RereleaseGame
            } else {
                NativeModuleApi::Q2ClassicGame
            },
        }]
    } else if q3_guest {
        vec![ExecutionModule::Qvm {
            owner: selection.source.clone(),
            artifact: ResourceRequest {
                content: product.id.clone(),
                path: "vm/qagame.qvm".to_string(),
            },
            role: ModuleRole::ServerGame,
            api: Q3ApiIdentity::Qagame(8),
        }]
    } else if quakeworld {
        vec![ExecutionModule::Quakec {
            owner: selection.source.clone(),
            artifact: ResourceRequest {
                content: product.id.clone(),
                path: "qwprogs.dat".to_string(),
            },
            api: QuakeCApiIdentity::Quakeworld,
        }]
    } else if let Some(program) = &native_program {
        vec![ExecutionModule::Quakec {
            owner: selection.source.clone(),
            artifact: ResourceRequest {
                content: product.id.clone(),
                path: program.clone(),
            },
            api: QuakeCApiIdentity::Netquake,
        }]
    } else {
        vec![typescript_execution(
            &selection.source,
            family,
            rerelease,
            if program_product.id == product.id {
                selection.source.provider.clone()
            } else {
                source_program_implementation(&program_product.expectation)
            },
        )]
    };
    Ok(LaunchPreset {
        id,
        weapon_behaviors: Vec::new(),
        mods: Vec::new(),
        map: MapSelection {
            geometry: ResourceRequest {
                content: match &options.map_product {
                    Some(map_product) => catalog.require(map_product)?.id.clone(),
                    None => product.id.clone(),
                },
                path: options.map.clone(),
            },
            entities: selection.source.clone(),
        },
        campaign: if options.mode == GameMode::Deathmatch {
            CampaignSelection::None
        } else {
            CampaignSelection::Campaign {
                mission: selection.source.clone(),
                gamecode: selection.source.clone(),
            }
        },
        movement: movement.clone(),
        character: CharacterSelection {
            definition: character.clone(),
            appearance,
        },
        weapons: vec![selection.source.clone()],
        equipment: native_equipment(catalog, &equipment_source, &selection.r#match)?,
        enemies: EnemySelection::MapDefined,
        presentation: PresentationSelection {
            doppler: DopplerSelection::Source,
            environment: EnvironmentSelection::AudioContent,
            assets: product.id.clone(),
            hud: selection.source.clone(),
            effects: selection.source.clone(),
            audio: selection.source.clone(),
        },
        engine_behavior: selection.source.clone(),
        combat: selection.source.clone(),
        inventory: selection.source.clone(),
        r#match: selection.r#match.clone(),
        transition: selection.source.clone(),
        execution,
        timing: vec![
            provider_timing,
            timing(&movement, content_family(options.movement), movement_edition, true)?,
            timing(&character, content_family(options.character), character_edition, false)?,
        ],
        ordering: FrameOrdering::Mixed {
            providers: vec![
                selection.source.provider.clone(),
                movement.provider.clone(),
                character.provider.clone(),
            ],
        },
    })
}

fn parent_dir(path: &str) -> &str {
    Path::new(path)
        .parent()
        .and_then(|parent| parent.to_str())
        .filter(|parent| !parent.is_empty())
        .unwrap_or(".")
}

fn file_name(path: &str) -> &str {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
}

fn q3_mount_options(q3_product: Option<Q3ApplicationProduct>) -> OpenMountOptions {
    let mut options = OpenMountOptions::default();
    if matches!(
        q3_product,
        Some(product) if matches!(product.restriction, Q3MountRestriction::Demo { .. })
    ) {
        options.q3_restriction = Some(Q3Restriction::Demo);
    }
    options
}

fn user_root(options: &ApplicationContentOptions) -> PathBuf {
    options
        .base
        .user_content_root
        .clone()
        .map(PathBuf::from)
        .unwrap_or_else(default_user_content_root)
}

fn provider_plan_id(namespace: &str, content: &ContentId) -> Result<qa_content::contract::MountPlanId, ContentError> {
    Ok(create_mount_plan_id(
        namespace,
        &hex_lower(content.as_str().as_bytes()),
    )?)
}

/// Opened remote content (`RemoteContentMounts`).
#[derive(Debug)]
pub struct RemoteContentMounts {
    /// Remote selection.
    pub selection: RemoteContentSelection,
    /// Discovered catalog.
    pub catalog: InstalledCatalog,
    /// Remote product.
    pub product: CatalogProduct,
    /// Opened mounts.
    pub mounts: MountedContent,
    /// Writable download root.
    pub write_root: String,
    /// Writable base root.
    pub base_write_root: String,
    /// Q3 application product.
    pub q3_product: Option<Q3ApplicationProduct>,
}

/// Open remote content (`openRemoteContent`).
pub fn open_remote_content<Q>(
    roots: &ApplicationContentOptions,
    requested: &RemoteContentSelection,
    assert_current: &dyn Fn(),
    generation: u64,
    q3: &mut Q,
) -> Result<RemoteContentMounts, ContentError>
where
    Q: Q3ProductPreparer,
    Q::Error: Into<ContentError>,
{
    assert_current();
    let selection = remote_content_selection(requested.base, &requested.directory)?;
    let base = expected_products()
        .into_iter()
        .find(|product| product.id == selection.base.as_str())
        .ok_or_else(|| ContentError::MissingRemoteBase(selection.base.as_str().to_string()))?;
    let user_content_root = user_root(roots);
    let family_dir = parent_dir(&base.content_directory);
    let family_root = find_content_path(&user_content_root, family_dir, PathComparison::CaseInsensitive)?
        .unwrap_or(user_product_directory(&user_content_root, family_dir)?);
    assert_current();
    let base_name = file_name(&base.content_directory);
    let base_write_root = find_content_path(&family_root, base_name, PathComparison::CaseInsensitive)?
        .unwrap_or(family_root.join(base_name));
    assert_current();
    let write_root = if selection.directory == base_name.to_lowercase() {
        base_write_root.clone()
    } else {
        find_content_path(&family_root, &selection.directory, PathComparison::CaseInsensitive)?
            .unwrap_or(family_root.join(&selection.directory))
    };
    assert_current();
    std::fs::create_dir_all(&write_root).map_err(|error| ContentError::Io {
        path: write_root.display().to_string(),
        message: error.to_string(),
    })?;
    assert_current();
    std::fs::create_dir_all(&base_write_root).map_err(|error| ContentError::Io {
        path: base_write_root.display().to_string(),
        message: error.to_string(),
    })?;
    assert_current();
    let mut catalog = discover_installed_content(&DiscoverContentOptions {
        corpus_root: PathBuf::from(&roots.base.corpus_root),
        user_content_root: Some(user_content_root),
        products: None,
        generation,
        discover_mods: false,
        remote_content: Some(selection.clone()),
    })?;
    assert_current();
    catalog.require(selection.base.as_str())?;
    let product_id = remote_content_product(&selection, None)?;
    let mut product = catalog.require(&product_id)?.clone();
    let owner = product
        .user_content
        .as_ref()
        .map(|user| user.root.clone())
        .or(product.loose_root.clone());
    if owner.as_deref() != Some(write_root.to_string_lossy().as_ref()) {
        return Err(ContentError::RemoteDirectoryMismatch);
    }
    let policy = q3
        .prepare_q3_application_product(catalog, product.id.as_str(), roots)
        .map_err(Into::into)?;
    assert_current();
    catalog = policy.catalog;
    product = catalog.require(&remote_content_product(&selection, None)?)?.clone();
    let mounts = catalog.mounts_for(product.id.as_str())?;
    assert_current();
    let plan = ResolvedMountPlan {
        id: create_mount_plan_id("remote-server", &generation.to_string())?,
        default_order: mounts.iter().map(|mount| mount.identity().id.clone()).collect(),
        mounts,
        prefix_orders: Vec::new(),
    };
    let opened = open_mount_plan(&plan, q3_mount_options(policy.q3_product))?;
    assert_current();
    Ok(RemoteContentMounts {
        selection,
        catalog,
        product,
        mounts: opened,
        write_root: write_root.display().to_string(),
        base_write_root: base_write_root.display().to_string(),
        q3_product: policy.q3_product,
    })
}

/// Mounted remote application content (`MountedApplicationContent`).
#[derive(Debug)]
pub struct MountedApplicationContent {
    /// Discovered catalog.
    pub catalog: InstalledCatalog,
    /// Opened mounts.
    pub mounts: MountedContent,
    /// Q3 application product.
    pub q3_product: Option<Q3ApplicationProduct>,
}

impl MountedApplicationContent {
    /// Close the mounts.
    pub fn close(self) {
        self.mounts.close();
    }
}

/// Open remote application content (`openRemoteApplicationContent`).
pub fn open_remote_application_content<Q>(
    options: &ApplicationContentOptions,
    q3: &mut Q,
) -> Result<MountedApplicationContent, ContentError>
where
    Q: Q3ProductPreparer,
    Q::Error: Into<ContentError>,
{
    let selection = match &options.remote_content {
        Some(remote) => Some(remote.clone()),
        None => match &options.base.network {
            Network::QwClient { .. } => Some(remote_content_selection(RemoteContentBase::Q1Quakeworld, "qw")?),
            Network::Q3Client { .. } => Some(remote_content_selection(RemoteContentBase::Q3Baseq3, "baseq3")?),
            Network::Q2Client { .. } => Some(remote_content_selection(RemoteContentBase::Q2ClassicBaseq2, "baseq2")?),
            _ => None,
        },
    };
    if let Some(selection) = selection {
        let content = open_remote_content(options, &selection, &|| {}, 0, q3)?;
        return Ok(MountedApplicationContent {
            catalog: content.catalog,
            mounts: content.mounts,
            q3_product: content.q3_product,
        });
    }
    let discovered = discover_installed_content(&DiscoverContentOptions {
        corpus_root: PathBuf::from(&options.base.corpus_root),
        user_content_root: Some(user_root(options)),
        products: None,
        generation: 0,
        discover_mods: false,
        remote_content: None,
    })?;
    let policy = q3
        .prepare_q3_application_product(discovered, &options.base.product, options)
        .map_err(Into::into)?;
    let catalog = policy.catalog;
    let product = catalog.require(&options.base.product)?.clone();
    let mounts = catalog.mounts_for(product.id.as_str())?;
    let plan = ResolvedMountPlan {
        id: create_mount_plan_id("remote-connection", &options.base.product)?,
        default_order: mounts.iter().map(|mount| mount.identity().id.clone()).collect(),
        mounts,
        prefix_orders: Vec::new(),
    };
    let opened = open_mount_plan(&plan, q3_mount_options(policy.q3_product))?;
    Ok(MountedApplicationContent {
        catalog,
        mounts: opened,
        q3_product: policy.q3_product,
    })
}

/// Configuration selection (`ApplicationConfigurationSelection`).
#[derive(Debug, Clone, PartialEq)]
pub struct ApplicationConfigurationSelection {
    /// Source provider.
    pub source: ProviderReference,
    /// Engine behavior provider.
    pub engine_behavior: ProviderReference,
    /// Match provider.
    pub r#match: ProviderReference,
    /// Combat provider.
    pub combat: ProviderReference,
    /// Movement provider.
    pub movement: ProviderReference,
    /// Provider timing.
    pub timing: Vec<ProviderTiming>,
}

/// Configuration content (`ApplicationConfigurationContent`).
#[derive(Debug)]
pub struct ApplicationConfigurationContent {
    /// Q3 application product.
    pub q3_product: Option<Q3ApplicationProduct>,
    /// Installed catalog.
    pub catalog: InstalledCatalog,
    /// Configuration selection.
    pub selection: ApplicationConfigurationSelection,
    /// Opened mounts.
    pub mounts: MountedContent,
}

impl ApplicationConfigurationContent {
    /// Close the mounts.
    pub fn close(self) {
        self.mounts.close();
    }
}

/// Configuration request (`ApplicationConfigurationRequest`).
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum ApplicationConfigurationRequest {
    /// Resolve a preset choice.
    Launch {
        /// Launch preset.
        preset: LaunchPreset,
        /// Launch choice.
        choice: LaunchChoice,
    },
    /// Reuse an executable recipe.
    Recipe {
        /// Executable recipe.
        recipe: ExecutableRecipe,
    },
}

/// Open configuration content (`openApplicationConfigurationContent`).
pub fn open_application_configuration_content<Q>(
    catalog: InstalledCatalog,
    request: ApplicationConfigurationRequest,
    q3_product: Option<Q3ApplicationProduct>,
    q3: &mut Q,
) -> Result<ApplicationConfigurationContent, ContentError>
where
    Q: Q3ProductPreparer,
    Q::Error: Into<ContentError>,
{
    let entities_content = match &request {
        ApplicationConfigurationRequest::Recipe { recipe } => recipe.map.entities.content.as_str().to_string(),
        ApplicationConfigurationRequest::Launch { preset, .. } => preset.map.entities.content.as_str().to_string(),
    };
    let minimal = ApplicationContentOptions {
        base: ApplicationOptions::default(),
        remote_content: None,
        q3_product,
    };
    let policy = q3
        .prepare_q3_application_product(catalog, &entities_content, &minimal)
        .map_err(Into::into)?;
    let catalog = policy.catalog;
    let (selection, plan) = match request {
        ApplicationConfigurationRequest::Recipe { recipe } => (
            ApplicationConfigurationSelection {
                source: recipe.map.entities.clone(),
                engine_behavior: recipe.engine_behavior.clone(),
                r#match: recipe.r#match.clone(),
                combat: recipe.combat.clone(),
                movement: recipe.movement.clone(),
                timing: recipe.timing.clone(),
            },
            recipe.mounts.clone(),
        ),
        ApplicationConfigurationRequest::Launch { preset, choice } => {
            let prepared = prepare_launch_mount_plan(
                &ResolveLaunchOptions {
                    choice: &choice,
                    preset: &preset,
                    catalog: &catalog,
                    id: None,
                    mounts: None,
                },
                &CatalogWeaponSources,
            )?;
            let selected = prepared.selected;
            (
                ApplicationConfigurationSelection {
                    source: selected.map.entities.clone(),
                    engine_behavior: selected.engine_behavior.clone(),
                    r#match: selected.r#match.clone(),
                    combat: selected.combat.clone(),
                    movement: selected.movement.clone(),
                    timing: selected.timing.clone(),
                },
                prepared.plan,
            )
        }
    };
    let mounts = open_mount_plan(&plan, q3_mount_options(policy.q3_product))?;
    Ok(ApplicationConfigurationContent {
        q3_product: policy.q3_product,
        catalog,
        selection,
        mounts,
    })
}

/// Build configuration content from mounted remote content
/// (`remoteConfigurationContent`).
pub fn remote_configuration_content(
    options: &ApplicationOptions,
    content: MountedApplicationContent,
) -> Result<ApplicationConfigurationContent, ContentError> {
    let preset = application_configuration_preset(&content.catalog, options, None)?;
    let selection = ApplicationConfigurationSelection {
        source: preset.map.entities.clone(),
        engine_behavior: preset.engine_behavior.clone(),
        r#match: preset.r#match.clone(),
        combat: preset.combat.clone(),
        movement: preset.movement.clone(),
        timing: preset.timing.clone(),
    };
    let MountedApplicationContent {
        catalog,
        mounts,
        q3_product,
    } = content;
    Ok(ApplicationConfigurationContent {
        q3_product,
        catalog,
        selection,
        mounts,
    })
}

/// Mod preparation purpose (`"gameplay" | "presentation"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModPurpose {
    /// Gameplay mods.
    Gameplay,
    /// Presentation-only mods.
    Presentation,
}

/// Quake II guest edition (`"classic" | "rerelease"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2Edition {
    /// Classic.
    Classic,
    /// Rerelease.
    Rerelease,
}

/// Configured recipe (`prepareConfiguredApplicationRecipe` result).
#[derive(Debug, Clone)]
pub struct ConfiguredApplicationRecipe {
    /// Installed catalog.
    pub catalog: InstalledCatalog,
    /// Configured recipe.
    pub recipe: ExecutableRecipe,
}

/// Scoped per-content mounts for lane preparation (`forContent`).
pub trait ContentScope {
    /// Open (or reuse) mounts scoped to a content identity.
    fn for_content(&mut self, content: &ContentId) -> Result<Rc<MountedContent>, ContentError>;
}

/// Lane preparation seam (donor `mod-selection.ts`,
/// `weapon-behavior-selection.ts`, `qvm-grapple-selection.ts`,
/// `simulation/quakec-source.ts`, `simulation/q3/guest-artifact.ts`,
/// `simulation/classic-guest-source.ts`,
/// `simulation/rerelease-guest-source.ts`, `simulation/native-q2-map.ts`).
pub trait ApplicationContentPreparer {
    /// Preparation failure.
    type Error;
    /// Prepared mod.
    type Mod;
    /// Prepared weapon behavior.
    type WeaponBehavior;
    /// Prepared QVM grapple.
    type QvmGrapple;
    /// Prepared QuakeC source.
    type QuakeC;
    /// Prepared Q3 game.
    type Q3Game;
    /// Prepared Quake II guest.
    type Q2Game;

    /// Apply configured mods and weapon behavior (`prepareConfiguredApplicationRecipe`).
    fn prepare_configured_recipe(
        &mut self,
        catalog: InstalledCatalog,
        options: &ApplicationContentOptions,
        recipe: ExecutableRecipe,
    ) -> Result<ConfiguredApplicationRecipe, Self::Error>;

    /// Prepare mods (`prepareApplicationMods`).
    fn prepare_mods(
        &mut self,
        catalog: &InstalledCatalog,
        recipe: &ExecutableRecipe,
        scope: &mut dyn ContentScope,
        purpose: ModPurpose,
    ) -> Result<Vec<Self::Mod>, Self::Error>;

    /// Prepare one weapon behavior (`prepareApplicationWeaponBehavior`).
    fn prepare_weapon_behavior(
        &mut self,
        catalog: &InstalledCatalog,
        selection: &ResolvedWeaponBehaviorSelection,
        scope: &mut dyn ContentScope,
    ) -> Result<Self::WeaponBehavior, Self::Error>;

    /// Prepare the QVM grapple (`prepareApplicationQvmGrapple`).
    fn prepare_qvm_grapple(
        &mut self,
        selection: &GrappleSelection,
        scope: &mut dyn ContentScope,
    ) -> Result<Self::QvmGrapple, Self::Error>;

    /// Prepare a QuakeC source (`prepareQuakeCSource`).
    fn prepare_quakec_source(
        &mut self,
        execution: &ResolvedExecutionModule,
        mounts: &MountedContent,
        entities: &str,
        source: Option<&MountedContent>,
    ) -> Result<Self::QuakeC, Self::Error>;

    /// Whether the QuakeC weapon stage qualifies mixed arsenals
    /// (`preparedQuakeCWeaponStage`).
    fn quakec_weapon_stage_qualified(&self, prepared: &Self::QuakeC) -> bool;

    /// Whether the QuakeC damage scaling qualifies mixed arsenals
    /// (`preparedQuakeCDamageScaling`).
    fn quakec_damage_scaling_qualified(&self, prepared: &Self::QuakeC) -> bool;

    /// Prepare a Q3 game (`prepareQ3Game`).
    fn prepare_q3_game(
        &mut self,
        execution: &ResolvedExecutionModule,
        mounts: &MountedContent,
    ) -> Result<Self::Q3Game, Self::Error>;

    /// Whether the Q3 primary interfaces are complete.
    fn q3_primary_complete(&self, prepared: &Self::Q3Game) -> bool;

    /// Prepare a Quake II guest (`prepareClassicGuest` / `prepareRereleaseGuest`).
    fn prepare_q2_guest(
        &mut self,
        execution: &ResolvedExecutionModule,
        mounts: &MountedContent,
        edition: Q2Edition,
    ) -> Result<Self::Q2Game, Self::Error>;

    /// Whether the native primary declaration is present (`preparedNativePrimary`).
    fn native_primary_present(&self, prepared: &Self::Q2Game) -> bool;

    /// Validate a native Quake II map (`prepareNativeQ2Map`).
    fn prepare_native_q2_map(
        &mut self,
        world: &ApplicationWorld<'_>,
        edition: Q2Edition,
        mode: GameMode,
    ) -> Result<(), Self::Error>;
}

/// Mount lifecycle shared with main-mount leases.
#[derive(Debug, Default)]
struct MountLifecycle {
    /// Whether the content closed.
    closed: Cell<bool>,
    /// Outstanding main-mount leases.
    main_mount_leases: Cell<usize>,
}

/// Borrowed pieces resolving scoped mounts.
struct ContentScopeRef<'a> {
    /// Installed catalog.
    catalog: &'a InstalledCatalog,
    /// Executable recipe.
    recipe: &'a ExecutableRecipe,
    /// Main mounts.
    mounts: &'a MountedContent,
    /// Pure-server policy.
    pure: &'a Option<PureMountPolicy>,
    /// Q3 application product.
    q3_product: &'a Option<Q3ApplicationProduct>,
    /// Mount lifecycle.
    lifecycle: &'a Rc<MountLifecycle>,
    /// Scoped mounts by content.
    scoped: &'a mut HashMap<ContentId, Rc<MountedContent>>,
    /// Opened scoped mounts.
    opened: &'a mut Vec<Rc<MountedContent>>,
}

impl ContentScope for ContentScopeRef<'_> {
    fn for_content(&mut self, content: &ContentId) -> Result<Rc<MountedContent>, ContentError> {
        if self.lifecycle.closed.get() {
            return Err(ContentError::Closed);
        }
        if let Some(existing) = self.scoped.get(content) {
            return Ok(Rc::clone(existing));
        }
        let primary = self.catalog.mounts_for(content.as_str())?;
        let rules = if *content == self.recipe.map.entities.content && self.recipe.r#match.content != *content {
            self.catalog.mounts_for(self.recipe.r#match.content.as_str())?
        } else {
            Vec::new()
        };
        let mut ordered: Vec<ContentMount> = Vec::new();
        let mut index = HashMap::new();
        for mount in primary.into_iter().chain(rules) {
            let id = mount.identity().id.clone();
            match index.get(&id) {
                Some(&slot) => ordered[slot] = mount,
                None => {
                    index.insert(id, ordered.len());
                    ordered.push(mount);
                }
            }
        }
        let digests: HashSet<&qa_content::contract::ContentDigest> = ordered
            .iter()
            .filter_map(|mount| match mount {
                ContentMount::Archive(archive) => Some(&archive.archive_digest),
                ContentMount::Loose(_) => None,
            })
            .collect();
        let pure = match self.pure {
            None => None,
            Some(policy) => {
                let archives = policy
                    .archives
                    .iter()
                    .filter(|digest| digests.contains(digest))
                    .cloned()
                    .collect::<Vec<_>>();
                if !policy.archives.is_empty() && archives.is_empty() {
                    return Err(ContentError::NoPureArchives(content.as_str().to_string()));
                }
                Some(PureMountPolicy { archives })
            }
        };
        let plan = ResolvedMountPlan {
            id: provider_plan_id("provider", content)?,
            default_order: ordered.iter().map(|mount| mount.identity().id.clone()).collect(),
            mounts: ordered,
            prefix_orders: Vec::new(),
        };
        let options = OpenMountOptions {
            pure,
            ..q3_mount_options(*self.q3_product)
        };
        let opened = match self.mounts.borrow_mount_plan(&plan, options.clone())? {
            Some(borrowed) => borrowed,
            None => open_mount_plan(&plan, options)?,
        };
        if self.lifecycle.closed.get() {
            opened.close();
            return Err(ContentError::ClosedDuringMount);
        }
        let shared = Rc::new(opened);
        self.scoped.insert(content.clone(), Rc::clone(&shared));
        self.opened.push(Rc::clone(&shared));
        Ok(shared)
    }
}

/// Loaded application content (`LoadedApplicationContent`).
pub struct LoadedApplicationContent<'a, P: ApplicationContentPreparer> {
    /// Installed catalog.
    pub catalog: InstalledCatalog,
    /// Executable recipe.
    pub recipe: ExecutableRecipe,
    /// Decoded world.
    pub world: ApplicationWorld<'a>,
    /// Main mounts.
    pub mounts: Rc<MountedContent>,
    /// Prepared QuakeC source.
    pub prepared_quakec: Option<P::QuakeC>,
    /// Pure-server policy.
    pure: Option<PureMountPolicy>,
    /// Prepared Q3 game.
    pub prepared_q3_game: Option<P::Q3Game>,
    /// Prepared Quake II guest.
    pub prepared_q2_game: Option<P::Q2Game>,
    /// Q3 application product.
    pub q3_product: Option<Q3ApplicationProduct>,
    /// Map sidecars sorted by path.
    pub map_sidecars: Vec<ApplicationMapSidecar>,
    /// Prepared mods.
    mod_owners: Vec<P::Mod>,
    /// Prepared QVM grapple.
    grapple_owner: Option<P::QvmGrapple>,
    /// Prepared weapon behaviors.
    weapon_behavior_owners: Vec<P::WeaponBehavior>,
    /// Scoped mounts by content.
    scoped: HashMap<ContentId, Rc<MountedContent>>,
    /// Mount lifecycle.
    lifecycle: Rc<MountLifecycle>,
    /// Opened scoped mounts.
    opened: Vec<Rc<MountedContent>>,
}

impl<P: ApplicationContentPreparer> std::fmt::Debug for LoadedApplicationContent<'_, P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedApplicationContent")
            .field("catalog", &self.catalog)
            .field("recipe", &self.recipe)
            .field("world", &self.world)
            .field("mounts", &self.mounts)
            .field("prepared_quakec", &self.prepared_quakec.is_some())
            .field("pure", &self.pure)
            .field("prepared_q3_game", &self.prepared_q3_game.is_some())
            .field("prepared_q2_game", &self.prepared_q2_game.is_some())
            .field("q3_product", &self.q3_product)
            .field("map_sidecars", &self.map_sidecars)
            .field("mods", &self.mod_owners.len())
            .field("grapple", &self.grapple_owner.is_some())
            .field("weapon_behaviors", &self.weapon_behavior_owners.len())
            .field("scoped", &self.scoped.keys().collect::<Vec<_>>())
            .field("lifecycle", &self.lifecycle)
            .field("opened", &self.opened.len())
            .finish()
    }
}

impl<'a, P: ApplicationContentPreparer> LoadedApplicationContent<'a, P> {
    /// Borrow the pieces resolving scoped mounts.
    fn scope(&mut self) -> ContentScopeRef<'_> {
        ContentScopeRef {
            catalog: &self.catalog,
            recipe: &self.recipe,
            mounts: &self.mounts,
            pure: &self.pure,
            q3_product: &self.q3_product,
            lifecycle: &self.lifecycle,
            scoped: &mut self.scoped,
            opened: &mut self.opened,
        }
    }

    /// Prepared mods.
    #[must_use]
    pub fn prepared_mods(&self) -> &[P::Mod] {
        &self.mod_owners
    }

    /// Prepared QVM grapple.
    #[must_use]
    pub fn prepared_qvm_grapple(&self) -> Option<&P::QvmGrapple> {
        self.grapple_owner.as_ref()
    }

    /// Prepared weapon behaviors.
    #[must_use]
    pub fn prepared_weapon_behaviors(&self) -> &[P::WeaponBehavior] {
        &self.weapon_behavior_owners
    }

    /// Prepare mods (`prepareMods`).
    pub fn prepare_mods(&mut self, preparer: &mut P, purpose: ModPurpose) -> Result<(), ContentError>
    where
        P::Error: Into<ContentError>,
    {
        let mut scope = self.scope();
        let (catalog, recipe) = (scope.catalog, scope.recipe);
        let owners = preparer
            .prepare_mods(catalog, recipe, &mut scope, purpose)
            .map_err(Into::into)?;
        self.mod_owners = owners;
        Ok(())
    }

    /// Prepare the QVM grapple (`prepareQvmGrapple`).
    pub fn prepare_qvm_grapple(&mut self, preparer: &mut P) -> Result<(), ContentError>
    where
        P::Error: Into<ContentError>,
    {
        let enabled = matches!(
            self.recipe.equipment.grapple,
            GrappleSelection::Enabled {
                mechanic: GrappleMechanicDetail::Q3Qvm { .. },
                ..
            }
        );
        if enabled {
            let mut scope = self.scope();
            let selection = scope.recipe.equipment.grapple.clone();
            let grapple = preparer
                .prepare_qvm_grapple(&selection, &mut scope)
                .map_err(Into::into)?;
            self.grapple_owner = Some(grapple);
        }
        Ok(())
    }

    /// Prepare weapon behaviors (`prepareWeaponBehaviors`).
    pub fn prepare_weapon_behaviors(&mut self, preparer: &mut P) -> Result<(), ContentError>
    where
        P::Error: Into<ContentError>,
    {
        if !self.recipe.weapon_behaviors.is_empty()
            && self.recipe.execution.iter().any(|module| match module {
                ExecutionModule::Quakec { .. } => true,
                ExecutionModule::Typescript { .. } => false,
                ExecutionModule::Native { role, .. } | ExecutionModule::Qvm { role, .. } => {
                    *role == ModuleRole::ServerGame
                }
            })
        {
            return Err(ContentError::WeaponBehaviorHook);
        }
        let mut roles = HashSet::new();
        for selection in &self.recipe.weapon_behaviors {
            if !roles.insert(selection.definition.role) {
                return Err(ContentError::WeaponBehaviorConflict);
            }
        }
        for index in 0..self.recipe.weapon_behaviors.len() {
            let mut scope = self.scope();
            let catalog = scope.catalog;
            let selection = scope.recipe.weapon_behaviors[index].clone();
            let prepared = preparer
                .prepare_weapon_behavior(catalog, &selection, &mut scope)
                .map_err(Into::into)?;
            self.weapon_behavior_owners.push(prepared);
        }
        Ok(())
    }

    /// Retain the main mounts past close (`retainMainMounts`).
    pub fn retain_main_mounts(&self) -> Result<Box<dyn FnOnce()>, ContentError> {
        if self.lifecycle.closed.get() {
            return Err(ContentError::Closed);
        }
        let lifecycle = Rc::clone(&self.lifecycle);
        let mounts = Rc::clone(&self.mounts);
        lifecycle.main_mount_leases.set(lifecycle.main_mount_leases.get() + 1);
        Ok(Box::new(move || {
            lifecycle.main_mount_leases.set(lifecycle.main_mount_leases.get() - 1);
            if lifecycle.closed.get() && lifecycle.main_mount_leases.get() == 0 {
                mounts.close();
            }
        }))
    }

    /// Opened mounts (`openedMounts`).
    #[must_use]
    pub fn opened_mounts(&self) -> Vec<Rc<MountedContent>> {
        if self.lifecycle.closed.get() {
            return Vec::new();
        }
        std::iter::once(Rc::clone(&self.mounts))
            .chain(self.opened.iter().cloned())
            .collect()
    }

    /// Borrow the main and opened mounts (`openedMounts`, borrowed).
    #[must_use]
    pub fn opened_mount_refs(&self) -> Vec<&MountedContent> {
        if self.lifecycle.closed.get() {
            return Vec::new();
        }
        std::iter::once(self.mounts.as_ref())
            .chain(self.opened.iter().map(|mounts| mounts.as_ref()))
            .collect()
    }

    /// Open (or reuse) mounts scoped to a content identity (`forContent`).
    pub fn for_content(&mut self, content: &ContentId) -> Result<Rc<MountedContent>, ContentError> {
        self.scope().for_content(content)
    }

    /// Close the content and its scoped mounts (`close`).
    pub fn close(&mut self) {
        if self.lifecycle.closed.get() {
            return;
        }
        self.lifecycle.closed.set(true);
        if self.lifecycle.main_mount_leases.get() == 0 {
            self.mounts.close();
        }
        for content in self.scoped.values() {
            content.close();
        }
        self.scoped.clear();
        self.opened.clear();
    }
}

/// Rebuild application options from a recipe (`applicationOptionsForRecipe`).
pub fn application_options_for_recipe(
    options: &ApplicationOptions,
    catalog: &InstalledCatalog,
    recipe: &ExecutableRecipe,
) -> Result<ApplicationOptions, ContentError> {
    fn family(provider: &ProviderId) -> Result<GameFamily, ContentError> {
        match provider.namespace.as_str() {
            "q1" => Ok(GameFamily::Q1),
            "q2" => Ok(GameFamily::Q2),
            "q3" => Ok(GameFamily::Q3),
            _ => Err(ContentError::NoInputAdapter(provider_text(provider))),
        }
    }
    let character = family(&recipe.character.definition.provider)?;
    let appearance = &recipe.character.appearance.provider;
    let model = if appearance.namespace == character.to_string() {
        appearance.name.strip_prefix("model/")
    } else {
        None
    }
    .ok_or_else(|| ContentError::NoCharacterModel(provider_text(appearance)))?;
    let movement = family(&recipe.movement.provider)?;
    let rules = match provider_text(&recipe.r#match.provider).as_str() {
        "q2:ctf" => MatchRules::Ctf,
        "q2:lmctf" => MatchRules::Lmctf,
        "q2:tag" => MatchRules::Tag,
        "q2:deathball" => MatchRules::Deathball,
        "q1:horde" => MatchRules::Horde,
        _ => MatchRules::Standard,
    };
    Ok(ApplicationOptions {
        product: catalog
            .product(recipe.map.entities.content.as_str())?
            .expectation
            .id
            .clone(),
        map: recipe.map.geometry.requested_path.clone(),
        map_product: Some(
            catalog
                .product(recipe.map.geometry_content.as_str())?
                .expectation
                .id
                .clone(),
        ),
        movement: options_family(movement),
        movement_product: Some(
            catalog
                .product(recipe.movement.content.as_str())?
                .expectation
                .id
                .clone(),
        ),
        character: options_family(character),
        character_model: model.to_string(),
        rules: Some(rules),
        ..options.clone()
    })
}

/// Resolve travel to another map (`resolveApplicationTravel`).
pub fn resolve_application_travel<P: ApplicationContentPreparer>(
    content: &LoadedApplicationContent<'_, P>,
    path: &str,
) -> Result<ExecutableRecipe, ContentError> {
    let recipe = &content.recipe;
    let geometry = resolve_launch_resource(
        &content.catalog,
        &content.mounts,
        &ResourceRequest {
            content: recipe.map.geometry_content.clone(),
            path: path.to_string(),
        },
        LaunchResourceKind::Map,
    )?;
    let mut resources = recipe.resources.clone();
    for resource in &mut resources {
        if resource.id == recipe.map.geometry.id {
            *resource = geometry.clone();
        }
    }
    Ok(ExecutableRecipe {
        map: ResolvedMap {
            geometry,
            ..recipe.map.clone()
        },
        resources,
        ..recipe.clone()
    })
}

/// Assert a Q3 guest recipe (`assertQ3GuestRecipe`).
pub fn assert_q3_guest_recipe(
    recipe: &ExecutableRecipe,
    owner: &ProviderReference,
    api: &Q3ApiIdentity,
) -> Result<(), ContentError> {
    let same = |reference: &ProviderReference| {
        reference.content == recipe.map.entities.content && reference.provider == recipe.map.entities.provider
    };
    let appearance_ok = recipe.character.appearance.content == recipe.map.entities.content
        && recipe.character.appearance.provider.namespace == "q3"
        && recipe.character.appearance.provider.name.starts_with("model/");
    let timing_ok = recipe
        .timing
        .iter()
        .find(|timing| timing.provider == recipe.map.entities.provider)
        .is_some_and(|timing| matches!(timing.clock, ClockProfile::Q3 { .. }));
    if !matches!(api, Q3ApiIdentity::Qagame(_))
        || !same(owner)
        || recipe.execution.len() != 1
        || owner.provider != ProviderId::new("q3", "official")
        || !matches!(recipe.campaign, CampaignSelection::None)
        || recipe.movement.provider != ProviderId::new("q3", "movement")
        || recipe.movement.content != recipe.map.entities.content
        || recipe.character.definition.provider != ProviderId::new("q3", "character")
        || recipe.character.definition.content != recipe.map.entities.content
        || !appearance_ok
        || recipe.weapons.len() != 1
        || [
            &recipe.engine_behavior,
            &recipe.combat,
            &recipe.inventory,
            &recipe.r#match,
            &recipe.transition,
        ]
        .into_iter()
        .any(|reference| !same(reference))
        || !matches!(recipe.enemies, EnemySelection::MapDefined)
        || !timing_ok
    {
        return Err(ContentError::Q3GuestRecipe);
    }
    Ok(())
}

fn open_map_content(
    catalog: &InstalledCatalog,
    recipe: &ExecutableRecipe,
    q3_product: Option<Q3ApplicationProduct>,
) -> Result<MountedContent, ContentError> {
    let mounts = catalog.mounts_for(recipe.map.geometry_content.as_str())?;
    let plan = ResolvedMountPlan {
        id: provider_plan_id("map-sidecars", &recipe.map.geometry_content)?,
        default_order: mounts.iter().map(|mount| mount.identity().id.clone()).collect(),
        mounts,
        prefix_orders: Vec::new(),
    };
    Ok(open_mount_plan(&plan, q3_mount_options(q3_product))?)
}

fn sidecar_path(map: &str, extension: &str) -> String {
    map.strip_suffix(".bsp")
        .map(|stem| format!("{stem}.{extension}"))
        .unwrap_or_else(|| map.to_string())
}

fn module_kind(module: &ResolvedExecutionModule) -> &'static str {
    match module {
        ExecutionModule::Qvm { .. } => "qvm",
        ExecutionModule::Quakec { .. } => "quakec",
        ExecutionModule::Native { .. } => "native",
        ExecutionModule::Typescript { .. } => "typescript",
    }
}

fn module_role(module: &ResolvedExecutionModule) -> ModuleRole {
    match module {
        ExecutionModule::Quakec { .. } => ModuleRole::ServerGame,
        ExecutionModule::Typescript { role, .. }
        | ExecutionModule::Native { role, .. }
        | ExecutionModule::Qvm { role, .. } => *role,
    }
}

fn module_owner(module: &ResolvedExecutionModule) -> &ProviderReference {
    match module {
        ExecutionModule::Typescript { owner, .. }
        | ExecutionModule::Native { owner, .. }
        | ExecutionModule::Qvm { owner, .. }
        | ExecutionModule::Quakec { owner, .. } => owner,
    }
}

fn module_artifact_path(module: &ResolvedExecutionModule) -> &str {
    match module {
        ExecutionModule::Qvm { artifact, .. }
        | ExecutionModule::Native { artifact, .. }
        | ExecutionModule::Quakec { artifact, .. } => artifact.requested_path.as_str(),
        ExecutionModule::Typescript { .. } => "",
    }
}

fn module_role_text(role: ModuleRole) -> &'static str {
    match role {
        ModuleRole::ServerGame => "server-game",
        ModuleRole::ClientGame => "client-game",
        ModuleRole::Ui => "ui",
    }
}

struct MaterialStore {
    materials: HashMap<String, Vec<u8>>,
}

impl Q2MapResources for MaterialStore {
    fn read_material(&self, path: &str) -> Option<Vec<u8>> {
        self.materials.get(path).cloned()
    }
}

/// Caller-owned byte arena keeping decoded worlds alive.
#[derive(Debug, Default)]
pub struct WorldByteArena {
    /// Stored chunks.
    chunks: Vec<Vec<u8>>,
}

impl WorldByteArena {
    /// Store bytes, returning the chunk index.
    pub fn push(&mut self, bytes: Vec<u8>) -> usize {
        self.chunks.push(bytes);
        self.chunks.len() - 1
    }

    /// Borrow a stored chunk.
    #[must_use]
    pub fn chunk(&self, index: usize) -> &[u8] {
        &self.chunks[index]
    }
}

/// Load application content (`loadApplicationContent`).
#[allow(clippy::too_many_arguments)]
pub fn load_application_content<'a, P, Q>(
    options: &ApplicationContentOptions,
    restored_recipe: Option<ExecutableRecipe>,
    pure: Option<PureMountPolicy>,
    installed_catalog: Option<InstalledCatalog>,
    presentation_source: Option<ApplicationContentSource>,
    compat: &dyn LaunchQvmCompatibility,
    q3: &mut Q,
    preparer: &mut P,
    arena: &'a mut WorldByteArena,
) -> Result<LoadedApplicationContent<'a, P>, ContentError>
where
    P: ApplicationContentPreparer,
    P::Error: Into<ContentError>,
    Q: Q3ProductPreparer,
    Q::Error: Into<ContentError>,
{
    let mut options = options.clone();
    if let Some(remote) = &options.remote_content {
        let (network_ok, recorded) = match remote.base {
            RemoteContentBase::Q1Quakeworld => {
                (matches!(options.base.network, Network::QwClient { .. }), DemoFamily::Qw)
            }
            RemoteContentBase::Q2ClassicBaseq2 => {
                (matches!(options.base.network, Network::Q2Client { .. }), DemoFamily::Q2)
            }
            _ => (matches!(options.base.network, Network::Q3Client { .. }), DemoFamily::Q3),
        };
        let context_ok = match presentation_source {
            None => network_ok,
            Some(source) => source.family == recorded,
        };
        if !context_ok || options.base.product != remote_content_product(remote, None)? {
            return Err(ContentError::RemoteProductMismatch);
        }
    }
    let mut catalog = match installed_catalog {
        Some(catalog) => catalog,
        None => discover_installed_content(&DiscoverContentOptions {
            corpus_root: PathBuf::from(&options.base.corpus_root),
            user_content_root: Some(user_root(&options)),
            products: None,
            generation: 0,
            discover_mods: application_discovers_mods(&options.base, restored_recipe.as_ref()),
            remote_content: options.remote_content.clone(),
        })?,
    };
    let restored_entities = restored_recipe
        .as_ref()
        .map(|recipe| recipe.map.entities.content.as_str().to_string());
    let product = restored_entities.as_deref().unwrap_or(&options.base.product);
    let prepared_product = q3
        .prepare_q3_application_product(catalog, product, &options)
        .map_err(Into::into)?;
    catalog = prepared_product.catalog;
    if let Some(q3_product) = prepared_product.q3_product {
        options.q3_product = Some(q3_product);
    }
    let mount_options = OpenMountOptions {
        pure: pure.clone(),
        ..q3_mount_options(options.q3_product)
    };
    if let Some(source) = presentation_source {
        let recorded = match source.family {
            DemoFamily::Q1 | DemoFamily::Qw => GameFamily::Q1,
            DemoFamily::Q2 => GameFamily::Q2,
            DemoFamily::Q3 => GameFamily::Q3,
        };
        if catalog.require(&options.base.product)?.expectation.family != recorded {
            return Err(ContentError::RecordedFamilyMismatch);
        }
    }
    let from_scratch = restored_recipe.is_none() && presentation_source.is_none();
    let mut recipe = match restored_recipe {
        Some(recipe) => recipe,
        None => {
            let preset = application_preset(&catalog, &options.base, None, presentation_source)?;
            let choice = preset_choice(preset.id.clone());
            resolve_launch(
                &ResolveLaunchOptions {
                    choice: &choice,
                    preset: &preset,
                    catalog: &catalog,
                    id: None,
                    mounts: Some(mount_options.clone()),
                },
                &CatalogWeaponSources,
                compat,
            )?
        }
    };
    if from_scratch {
        let configured = preparer
            .prepare_configured_recipe(catalog, &options, recipe)
            .map_err(Into::into)?;
        catalog = configured.catalog;
        recipe = configured.recipe;
    }
    let unified = presentation_source.is_some_and(|source| source.kind == ApplicationContentSourceKind::Unified);
    for module in &recipe.execution {
        if unified {
            continue;
        }
        match module {
            ExecutionModule::Qvm {
                owner,
                role: ModuleRole::ServerGame,
                api,
                ..
            } => {
                if (!options.base.dedicated && !matches!(options.base.network, Network::Offline))
                    || (!matches!(options.base.network, Network::NativeServer { .. })
                        && !matches!(options.base.network, Network::Offline))
                    || options.base.mode != GameMode::Deathmatch
                {
                    return Err(ContentError::QvmRejected);
                }
                assert_q3_guest_recipe(&recipe, owner, api)?;
            }
            ExecutionModule::Quakec { owner, api, .. } => {
                let entities = recipe.map.entities.content.as_str().to_string();
                let product = catalog.product(&entities)?.expectation.clone();
                let native_qw = product.edition == "quakeworld"
                    && matches!(api, QuakeCApiIdentity::Quakeworld)
                    && options.base.mode == GameMode::Deathmatch
                    && matches!(options.base.network, Network::Offline | Network::NativeServer { .. })
                    && options.base.q1_protocol.is_none();
                let native_nq = catalog.product(&entities)?.expectation.family == GameFamily::Q1
                    && matches!(api, QuakeCApiIdentity::Netquake)
                    && matches!(options.base.network, Network::Offline | Network::NativeServer { .. });
                if native_qw && !options.base.dedicated
                    || !native_qw && !native_nq
                    || !matches!(options.base.network, Network::Offline)
                        && recipe.map.geometry_content != recipe.map.entities.content
                    || owner.provider != recipe.map.entities.provider
                    || owner.content != recipe.map.entities.content
                    || recipe.execution.len() != 1
                {
                    return Err(ContentError::QuakeCModuleRejected);
                }
            }
            ExecutionModule::Native {
                owner,
                role: ModuleRole::ServerGame,
                api,
                profile,
                ..
            } if matches!(
                (api, profile),
                (NativeModuleApi::Q2ClassicGame, NativeAbi::WindowsI386)
                    | (NativeModuleApi::Q2RereleaseGame, NativeAbi::WindowsX86_64)
            ) =>
            {
                let product = catalog.product(recipe.map.entities.content.as_str())?;
                let rerelease = matches!(api, NativeModuleApi::Q2RereleaseGame);
                if product.expectation.family != GameFamily::Q2
                    || rerelease && product.expectation.edition != "rerelease"
                    || !rerelease && product.expectation.edition != "classic"
                    || recipe.execution.len() != 1
                    || owner.provider != recipe.map.entities.provider
                    || owner.content != recipe.map.entities.content
                    || recipe.movement.provider.namespace != "q2"
                    || recipe.movement.provider.name != "movement"
                    || recipe.character.definition.provider.namespace != "q2"
                    || recipe.character.definition.provider.name != "character"
                    || !matches!(recipe.enemies, EnemySelection::MapDefined)
                {
                    return Err(ContentError::NativeQ2Rejected);
                }
            }
            ExecutionModule::Typescript { .. } => {}
            module => {
                return Err(ContentError::UnsupportedModule(format!(
                    "Application cannot execute {} {} module {} ({}): this executor is not joined to the shared simulation. Select a supported TypeScript execution module.",
                    module_kind(module),
                    module_role_text(module_role(module)),
                    provider_text(&module_owner(module).provider),
                    module_artifact_path(module)
                )));
            }
        }
    }
    let mounts = open_mount_plan(&recipe.mounts, mount_options)?;
    if pure.is_some() {
        let geometry = resolve_launch_resource(
            &catalog,
            &mounts,
            &ResourceRequest {
                content: recipe.map.geometry_content.clone(),
                path: recipe.map.geometry.requested_path.clone(),
            },
            LaunchResourceKind::Map,
        )?;
        let old = recipe.map.geometry.id.clone();
        recipe.mounts = mounts.plan.clone();
        recipe.map.geometry = geometry.clone();
        for resource in &mut recipe.resources {
            if resource.id == old {
                *resource = geometry.clone();
            }
        }
    }
    let geometry_bytes = mounts.read(ResourceRef::Resolved(&recipe.map.geometry))?;
    let map = recipe.map.geometry.requested_path.clone();
    let family = classify_bsp(&geometry_bytes, &map)?;
    let geometry_index = arena.push(geometry_bytes);
    let frozen: &'a WorldByteArena = arena;
    let geometry: &'a [u8] = frozen.chunk(geometry_index);
    let mut map_sidecars: Vec<ApplicationMapSidecar> = Vec::new();
    let world = match family {
        BspKind::Q1 => {
            let entities_path = sidecar_path(&map, "ent");
            let lit_path = sidecar_path(&map, "lit");
            let (entities, lit) = {
                let map_content = open_map_content(&catalog, &recipe, options.q3_product)?;
                let entities = map_content.open(&entities_path, |_| true)?;
                let lit = map_content.open(&lit_path, |_| true)?;
                map_sidecars.push(ApplicationMapSidecar {
                    content: recipe.map.geometry_content.clone(),
                    path: entities_path,
                    resource: entities.as_ref().map(|found| found.reference.clone()),
                });
                map_sidecars.push(ApplicationMapSidecar {
                    content: recipe.map.geometry_content.clone(),
                    path: lit_path,
                    resource: lit.as_ref().map(|found| found.reference.clone()),
                });
                (entities, lit)
            };
            // Donor `loadApplicationContent`: missing sidecars stay absent from
            // the reader options (`...(lit === null ? {} : { lit: lit.bytes })`).
            let bsp_options = Q1BspOptions {
                entities: entities.as_ref().map(|found| found.bytes.as_slice()),
                lit: lit.as_ref().map(|found| found.bytes.as_slice()),
            };
            ApplicationWorld::Q1(read_q1_bsp(geometry, &map, bsp_options)?)
        }
        BspKind::Q2 => {
            let raw = read_q2_bsp(geometry, &map)?;
            let mut seen = HashSet::new();
            let mut materials = HashMap::new();
            {
                let map_content = open_map_content(&catalog, &recipe, options.q3_product)?;
                for texture in &raw.texture_info {
                    let path = format!("textures/{}.mat", texture.name);
                    if seen.insert(path.clone()) {
                        let asset = map_content.open(&path, |_| true)?;
                        map_sidecars.push(ApplicationMapSidecar {
                            content: recipe.map.geometry_content.clone(),
                            path: path.clone(),
                            resource: asset.as_ref().map(|found| found.reference.clone()),
                        });
                        if let Some(asset) = asset {
                            materials.insert(path, asset.bytes);
                        }
                    }
                }
            }
            let store = MaterialStore { materials };
            ApplicationWorld::Q2(to_q2_world_geometry(raw, Some(&store))?)
        }
        BspKind::Q3 => ApplicationWorld::Q3(decode_q3_world(geometry, &map)?),
    };
    let quakec = recipe.execution.iter().find_map(|module| match module {
        ExecutionModule::Quakec { owner, .. } => Some((module, owner)),
        _ => None,
    });
    if quakec.is_some() && matches!(options.base.network, Network::NativeServer { .. }) && world.kind() != "q1-bsp" {
        return Err(ContentError::NativeClientGeometry);
    }
    let mut prepared_quakec = None;
    if !unified {
        if let Some((execution, owner)) = quakec {
            if recipe.map.geometry_content != owner.content {
                let source_mounts = catalog.mounts_for(owner.content.as_str())?;
                let plan = ResolvedMountPlan {
                    id: provider_plan_id("quakec-source", &owner.content)?,
                    default_order: source_mounts.iter().map(|mount| mount.identity().id.clone()).collect(),
                    mounts: source_mounts,
                    prefix_orders: Vec::new(),
                };
                let source_content = match mounts.borrow_mount_plan(&plan, OpenMountOptions::default())? {
                    Some(borrowed) => borrowed,
                    None => open_mount_plan(&plan, OpenMountOptions::default())?,
                };
                prepared_quakec = Some(
                    preparer
                        .prepare_quakec_source(execution, &mounts, world.entities(), Some(&source_content))
                        .map_err(Into::into)?,
                );
                source_content.close();
            } else {
                prepared_quakec = Some(
                    preparer
                        .prepare_quakec_source(execution, &mounts, world.entities(), None)
                        .map_err(Into::into)?,
                );
            }
        }
    }
    let mixed_arsenal = recipe
        .weapons
        .iter()
        .any(|weapon| weapon.provider != recipe.map.entities.provider || weapon.content != recipe.map.entities.content);
    if let Some(prepared) = &prepared_quakec {
        if mixed_arsenal {
            if !preparer.quakec_weapon_stage_qualified(prepared) {
                return Err(ContentError::QuakeCWeaponStage);
            }
            if !preparer.quakec_damage_scaling_qualified(prepared) {
                return Err(ContentError::QuakeCDamageScale);
            }
        }
    }
    let q3_execution = recipe.execution.iter().find(|module| {
        matches!(
            module,
            ExecutionModule::Qvm {
                role: ModuleRole::ServerGame,
                ..
            }
        )
    });
    let mut prepared_q3 = None;
    if !unified {
        if let Some(execution) = q3_execution {
            prepared_q3 = Some(preparer.prepare_q3_game(execution, &mounts).map_err(Into::into)?);
        }
    }
    if let Some(prepared) = &prepared_q3 {
        if mixed_arsenal && !preparer.q3_primary_complete(prepared) {
            return Err(ContentError::QvmArsenal);
        }
    }
    let q2_execution = recipe.execution.iter().find_map(|module| match module {
        ExecutionModule::Native {
            role: ModuleRole::ServerGame,
            api: NativeModuleApi::Q2RereleaseGame,
            ..
        } => Some((module, Q2Edition::Rerelease)),
        ExecutionModule::Native {
            role: ModuleRole::ServerGame,
            ..
        } => Some((module, Q2Edition::Classic)),
        _ => None,
    });
    let mut prepared_q2 = None;
    let mut q2_edition = None;
    if !unified {
        if let Some((execution, edition)) = q2_execution {
            prepared_q2 = Some(
                preparer
                    .prepare_q2_guest(execution, &mounts, edition)
                    .map_err(Into::into)?,
            );
            q2_edition = Some(edition);
        }
    }
    if let Some(prepared) = &prepared_q2 {
        if mixed_arsenal && !preparer.native_primary_present(prepared) {
            return Err(ContentError::NativeArsenal);
        }
    }
    if let Some(edition) = q2_edition {
        preparer
            .prepare_native_q2_map(&world, edition, options.base.mode)
            .map_err(Into::into)?;
    }
    map_sidecars.sort_by(|a, b| a.path.cmp(&b.path));
    let mut loaded = LoadedApplicationContent {
        catalog,
        recipe,
        world,
        mounts: Rc::new(mounts),
        prepared_quakec,
        pure,
        prepared_q3_game: prepared_q3,
        prepared_q2_game: prepared_q2,
        q3_product: options.q3_product,
        map_sidecars,
        mod_owners: Vec::new(),
        grapple_owner: None,
        weapon_behavior_owners: Vec::new(),
        scoped: HashMap::new(),
        lifecycle: Rc::new(MountLifecycle::default()),
        opened: Vec::new(),
    };
    let prepared = (|| -> Result<(), ContentError> {
        if presentation_source.is_none() {
            loaded.prepare_weapon_behaviors(preparer)?;
            loaded.prepare_mods(preparer, ModPurpose::Gameplay)?;
            loaded.prepare_qvm_grapple(preparer)?;
        } else if presentation_source.is_some_and(|source| source.kind == ApplicationContentSourceKind::Unified) {
            loaded.prepare_mods(preparer, ModPurpose::Presentation)?;
        }
        Ok(())
    })();
    if let Err(error) = prepared {
        loaded.close();
        return Err(error);
    }
    Ok(loaded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::catalog::{BehaviorMounts, ProductAvailability, QvmCompatRole};
    use qa_content::contract::{
        ContentDigest, LooseMount, MountId, MountIdentity, QvmAbiProfile, ResolvedResourceReference,
        ResourceProvenance, ResourceResolution,
    };
    use std::fs;

    fn stock_catalog() -> InstalledCatalog {
        let products = expected_products()
            .into_iter()
            .map(|expectation| CatalogProduct {
                id: ContentId(expectation.id.clone()),
                expectation,
                availability: ProductAvailability::Installed,
                archives: Vec::new(),
                loose_root: None,
                user_content: None,
                maps: Vec::new(),
                diagnostics: Vec::new(),
            })
            .collect();
        InstalledCatalog::new("corpus".to_string(), products, Vec::new(), 0, None).unwrap()
    }

    fn test_options() -> ApplicationOptions {
        ApplicationOptions {
            product: "q2-classic-baseq2".to_string(),
            map: "maps/base1.bsp".to_string(),
            ..ApplicationOptions::default()
        }
    }

    fn scratch_dir(name: &str) -> PathBuf {
        let root: PathBuf = std::env::temp_dir().join(format!("qa-content-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn q1_test_bsp() -> Vec<u8> {
        let mut bytes = vec![0u8; 4 + 15 * 8];
        bytes[0] = 29;
        bytes
    }

    #[test]
    fn q3_policy_matrix() {
        assert_eq!(q3_product_policy(false, false), Q3ProductPolicy::Retail);
        assert_eq!(
            q3_product_policy(true, false),
            Q3ProductPolicy::PrereleaseDemo {
                team_arena_ui: Q3TeamArenaUi::Retail
            }
        );
        assert_eq!(
            q3_product_policy(true, true),
            Q3ProductPolicy::PrereleaseDemo {
                team_arena_ui: Q3TeamArenaUi::Demo
            }
        );
        assert_eq!(q3_product_policy(false, true), Q3ProductPolicy::PrereleaseTaDemo);
        assert!(q3_prerelease_demo(q3_product_policy(true, false)));
        assert!(!q3_prerelease_demo(q3_product_policy(false, true)));
        assert!(q3_team_arena_demo(q3_product_policy(false, true)));
        assert!(q3_team_arena_demo(q3_product_policy(true, true)));
        assert!(!q3_team_arena_demo(q3_product_policy(true, false)));
        assert_eq!(
            q3_mount_restriction(Q3ProductPolicy::Retail, false),
            Q3MountRestriction::None
        );
        assert_eq!(
            q3_mount_restriction(Q3ProductPolicy::Retail, true),
            Q3MountRestriction::Demo {
                directory: "demota",
                pak_checksum: 437_558_517
            }
        );
        assert_eq!(q3_product_map_commands(Q3ProductPolicy::Retail).len(), 4);
        assert_eq!(q3_product_map_commands(q3_product_policy(true, false)), &["map"]);
    }

    #[test]
    fn source_selection_defaults_to_standard() {
        let catalog = stock_catalog();
        let selection = application_source_selection(&catalog, "q2-classic-baseq2", None).unwrap();
        assert_eq!(selection.source.provider, ProviderId::new("q2", "official"));
        assert_eq!(selection.source.content.as_str(), "q2-classic-baseq2");
        assert_eq!(selection.r#match, selection.source);
        assert_eq!(selection.rules, MatchRules::Standard);
    }

    #[test]
    fn source_selection_resolves_ctf_match() {
        let catalog = stock_catalog();
        let selection = application_source_selection(&catalog, "q2-classic-baseq2", Some(MatchRules::Ctf)).unwrap();
        assert_eq!(selection.r#match.provider, ProviderId::new("q2", "ctf"));
        assert_eq!(selection.r#match.content.as_str(), "q2-classic-ctf");
        assert_eq!(selection.rules, MatchRules::Ctf);
    }

    #[test]
    fn source_selection_rejects_bad_rules() {
        let catalog = stock_catalog();
        let error = application_source_selection(&catalog, "q1-classic-id1", Some(MatchRules::Ctf)).unwrap_err();
        assert!(matches!(error, ContentError::RulesNeedClassicQ2(_)));
        let error = application_source_selection(&catalog, "q2-classic-baseq2", Some(MatchRules::Tag)).unwrap_err();
        assert!(matches!(error, ContentError::RulesNeedRogue));
        let error = application_source_selection(&catalog, "q2-classic-baseq2", Some(MatchRules::Horde)).unwrap_err();
        assert!(matches!(error, ContentError::RulesNeedHorde));
    }

    #[test]
    fn discovers_mods_matrix() {
        let options = test_options();
        assert!(!application_discovers_mods(&options, None));
        let dedicated = ApplicationOptions {
            dedicated: true,
            ..options.clone()
        };
        assert!(application_discovers_mods(&dedicated, None));
        let q3 = ApplicationOptions {
            product: "q3-baseq3".to_string(),
            ..options.clone()
        };
        assert!(application_discovers_mods(&q3, None));
        let custom = ApplicationOptions {
            product: "custom-mod".to_string(),
            ..options
        };
        assert!(application_discovers_mods(&custom, None));
    }

    #[test]
    fn movement_resolution_paths() {
        let catalog = stock_catalog();
        let options = test_options();
        let same = resolve_application_movement(&catalog, &options).unwrap();
        assert_eq!(same.movement, OptionsFamily::Q1);
        let moved = ApplicationOptions {
            movement_product: Some("q2-classic-baseq2".to_string()),
            ..options
        };
        let resolved = resolve_application_movement(&catalog, &moved).unwrap();
        assert_eq!(resolved.movement, OptionsFamily::Q2);
        assert_eq!(resolved.movement_product.as_deref(), Some("q2-classic-baseq2"));
    }

    #[test]
    fn player_products_cover_override_and_base() {
        let catalog = stock_catalog();
        let options = ApplicationOptions {
            movement_product: Some("q2-classic-baseq2".to_string()),
            ..test_options()
        };
        let products = application_player_products(&catalog, &options, None).unwrap();
        assert_eq!(products.movement, "q2-classic-baseq2");
        assert_eq!(products.character, "q3-baseq3");
        let native = application_player_products(&catalog, &test_options(), Some(true)).unwrap();
        assert_eq!(native.movement, "q1-classic-id1");
    }

    #[test]
    fn preset_builds_typescript_launch() {
        let catalog = stock_catalog();
        let preset = application_preset(&catalog, &test_options(), None, None).unwrap();
        assert_eq!(preset.map.geometry.path, "maps/base1.bsp");
        assert_eq!(preset.map.geometry.content.as_str(), "q2-classic-baseq2");
        assert_eq!(preset.map.entities.provider, ProviderId::new("q2", "official"));
        assert_eq!(preset.execution.len(), 1);
        assert!(matches!(preset.execution[0], ExecutionModule::Typescript { .. }));
        assert_eq!(preset.timing.len(), 3);
        assert_eq!(preset.weapons.len(), 1);
        assert!(matches!(preset.campaign, CampaignSelection::Campaign { .. }));
        let deathmatch = application_preset(
            &catalog,
            &ApplicationOptions {
                mode: GameMode::Deathmatch,
                ..test_options()
            },
            None,
            None,
        )
        .unwrap();
        assert!(matches!(deathmatch.campaign, CampaignSelection::None));
        let configured = application_configuration_preset(&catalog, &test_options(), None).unwrap();
        assert_eq!(configured.map.entities, preset.map.entities);
    }

    #[test]
    fn preset_rejects_q3_guest_without_deathmatch() {
        let stock = stock_catalog();
        let mut expectation = stock.require("q3-baseq3").unwrap().expectation.clone();
        expectation.id = "custom-q3".to_string();
        expectation.edition = "custom".to_string();
        let products = expected_products()
            .into_iter()
            .map(|stock| CatalogProduct {
                id: ContentId(stock.id.clone()),
                expectation: stock,
                availability: ProductAvailability::Installed,
                archives: Vec::new(),
                loose_root: None,
                user_content: None,
                maps: Vec::new(),
                diagnostics: Vec::new(),
            })
            .chain(std::iter::once(CatalogProduct {
                id: ContentId("custom-q3".to_string()),
                expectation,
                availability: ProductAvailability::Installed,
                archives: Vec::new(),
                loose_root: None,
                user_content: None,
                maps: Vec::new(),
                diagnostics: Vec::new(),
            }))
            .collect();
        let catalog = InstalledCatalog::new("corpus".to_string(), products, Vec::new(), 0, None).unwrap();
        let options = ApplicationOptions {
            product: "custom-q3".to_string(),
            ..test_options()
        };
        let error = application_preset(&catalog, &options, None, None).unwrap_err();
        assert!(matches!(error, ContentError::Q3GuestRejected));
    }

    #[test]
    fn sidecar_paths_replace_trailing_bsp_only() {
        assert_eq!(sidecar_path("maps/test.bsp", "ent"), "maps/test.ent");
        assert_eq!(sidecar_path("maps/test.BSP", "ent"), "maps/test.BSP");
        assert_eq!(sidecar_path("maps/test", "lit"), "maps/test");
    }

    struct StubQ3;

    impl Q3ProductPreparer for StubQ3 {
        type Error = ContentError;

        fn prepare_q3_application_product(
            &mut self,
            catalog: InstalledCatalog,
            _product: &str,
            options: &ApplicationContentOptions,
        ) -> Result<PreparedQ3Product, ContentError> {
            Ok(PreparedQ3Product {
                catalog,
                q3_product: options.q3_product,
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
        ) -> Result<QvmAbiProfile, qa_content::catalog::CatalogError> {
            Ok(QvmAbiProfile::Modern)
        }
    }

    #[derive(Default)]
    struct StubPreparer {
        configured: usize,
        mods: usize,
        weapons: usize,
        grapples: usize,
        quakec: usize,
        q3_games: usize,
        q2_guests: usize,
        q2_maps: usize,
    }

    impl ApplicationContentPreparer for StubPreparer {
        type Error = ContentError;
        type Mod = String;
        type WeaponBehavior = String;
        type QvmGrapple = String;
        type QuakeC = String;
        type Q3Game = String;
        type Q2Game = String;

        fn prepare_configured_recipe(
            &mut self,
            catalog: InstalledCatalog,
            _options: &ApplicationContentOptions,
            recipe: ExecutableRecipe,
        ) -> Result<ConfiguredApplicationRecipe, ContentError> {
            self.configured += 1;
            Ok(ConfiguredApplicationRecipe { catalog, recipe })
        }

        fn prepare_mods(
            &mut self,
            _catalog: &InstalledCatalog,
            _recipe: &ExecutableRecipe,
            _scope: &mut dyn ContentScope,
            purpose: ModPurpose,
        ) -> Result<Vec<String>, ContentError> {
            self.mods += 1;
            Ok(vec![format!("{purpose:?}")])
        }

        fn prepare_weapon_behavior(
            &mut self,
            _catalog: &InstalledCatalog,
            _selection: &ResolvedWeaponBehaviorSelection,
            _scope: &mut dyn ContentScope,
        ) -> Result<String, ContentError> {
            self.weapons += 1;
            Ok("behavior".to_string())
        }

        fn prepare_qvm_grapple(
            &mut self,
            _selection: &GrappleSelection,
            _scope: &mut dyn ContentScope,
        ) -> Result<String, ContentError> {
            self.grapples += 1;
            Ok("grapple".to_string())
        }

        fn prepare_quakec_source(
            &mut self,
            _execution: &ResolvedExecutionModule,
            _mounts: &MountedContent,
            _entities: &str,
            _source: Option<&MountedContent>,
        ) -> Result<String, ContentError> {
            self.quakec += 1;
            Ok("quakec".to_string())
        }

        fn quakec_weapon_stage_qualified(&self, _prepared: &String) -> bool {
            true
        }

        fn quakec_damage_scaling_qualified(&self, _prepared: &String) -> bool {
            true
        }

        fn prepare_q3_game(
            &mut self,
            _execution: &ResolvedExecutionModule,
            _mounts: &MountedContent,
        ) -> Result<String, ContentError> {
            self.q3_games += 1;
            Ok("q3".to_string())
        }

        fn q3_primary_complete(&self, _prepared: &String) -> bool {
            true
        }

        fn prepare_q2_guest(
            &mut self,
            _execution: &ResolvedExecutionModule,
            _mounts: &MountedContent,
            _edition: Q2Edition,
        ) -> Result<String, ContentError> {
            self.q2_guests += 1;
            Ok("q2".to_string())
        }

        fn native_primary_present(&self, _prepared: &String) -> bool {
            true
        }

        fn prepare_native_q2_map(
            &mut self,
            _world: &ApplicationWorld<'_>,
            _edition: Q2Edition,
            _mode: GameMode,
        ) -> Result<(), ContentError> {
            self.q2_maps += 1;
            Ok(())
        }
    }

    fn scripted_reference(path: &str, content: &str) -> ResolvedResourceReference {
        use qa_content::contract::{MountPlanId, ResourceId};
        qa_content::contract::ResolvedResourceReference {
            id: ResourceId(format!("resource:test:{path}")),
            requested_path: path.to_string(),
            provenance: ResourceProvenance::Loose {
                mount: LooseMount {
                    identity: MountIdentity {
                        id: MountId("mount:test:loose".to_string()),
                        content: ContentId(content.to_string()),
                        generation: 0,
                    },
                    root_path: "corpus".to_string(),
                },
                member_path: path.to_string(),
            },
            digest: ContentDigest("sha256:00".to_string()),
            byte_length: 0,
            resolution: ResourceResolution::DefaultOrder {
                plan: MountPlanId("mount-plan:test:scripted".to_string()),
                rank: 0,
            },
        }
    }

    fn scripted_recipe(
        catalog: &InstalledCatalog,
        family: GameFamily,
        content: &str,
        geometry: ResolvedResourceReference,
        plan: ResolvedMountPlan,
        execution: Vec<ResolvedExecutionModule>,
    ) -> ExecutableRecipe {
        let source = ProviderReference {
            provider: ProviderId::new(&family.to_string(), "official"),
            content: ContentId(content.to_string()),
        };
        let movement = ProviderReference {
            provider: ProviderId::new(&family.to_string(), "movement"),
            content: ContentId(content.to_string()),
        };
        let character = ProviderReference {
            provider: ProviderId::new(&family.to_string(), "character"),
            content: ContentId(content.to_string()),
        };
        let appearance = ProviderReference {
            provider: ProviderId::new(&family.to_string(), "model/sarge"),
            content: ContentId(content.to_string()),
        };
        ExecutableRecipe {
            weapon_behaviors: Vec::new(),
            mods: Vec::new(),
            schema_version: 3,
            id: qa_content::contract::RecipeId("recipe:test:scripted".to_string()),
            preset: qa_content::contract::RecipeId("recipe:test:preset".to_string()),
            map: ResolvedMap {
                geometry_content: ContentId(content.to_string()),
                geometry: geometry.clone(),
                entities: source.clone(),
            },
            campaign: CampaignSelection::None,
            movement,
            character: CharacterSelection {
                definition: character,
                appearance,
            },
            weapons: vec![source.clone()],
            equipment: native_equipment(catalog, &source, &source).unwrap(),
            enemies: EnemySelection::MapDefined,
            presentation: PresentationSelection {
                doppler: DopplerSelection::Source,
                environment: EnvironmentSelection::AudioContent,
                assets: ContentId(content.to_string()),
                hud: source.clone(),
                effects: source.clone(),
                audio: source.clone(),
            },
            engine_behavior: source.clone(),
            combat: source.clone(),
            inventory: source.clone(),
            r#match: source.clone(),
            transition: source.clone(),
            execution,
            mounts: plan,
            resources: vec![geometry],
            timing: vec![native_provider_timing(&source, family, false)],
            ordering: FrameOrdering::Mixed {
                providers: vec![source.provider.clone()],
            },
        }
    }

    fn typescript_execution(owner: &ProviderReference) -> ResolvedExecutionModule {
        ExecutionModule::Typescript {
            owner: owner.clone(),
            implementation: ProviderId::new("q1", "official"),
            role: ModuleRole::ServerGame,
            api: SourceModuleApi::Quakec(QuakeCApiIdentity::Netquake),
        }
    }

    #[test]
    fn options_round_trip_through_recipe() {
        let catalog = stock_catalog();
        let geometry = scripted_reference("maps/base1.bsp", "q2-classic-baseq2");
        let plan = ResolvedMountPlan {
            id: create_mount_plan_id("test", "options").unwrap(),
            mounts: Vec::new(),
            default_order: Vec::new(),
            prefix_orders: Vec::new(),
        };
        let source = ProviderReference {
            provider: ProviderId::new("q2", "official"),
            content: ContentId("q2-classic-baseq2".to_string()),
        };
        let mut recipe = scripted_recipe(
            &catalog,
            GameFamily::Q2,
            "q2-classic-baseq2",
            geometry,
            plan,
            vec![typescript_execution(&source)],
        );
        recipe.r#match = ProviderReference {
            provider: ProviderId::new("q2", "ctf"),
            content: ContentId("q2-classic-ctf".to_string()),
        };
        let rebuilt = application_options_for_recipe(&test_options(), &catalog, &recipe).unwrap();
        assert_eq!(rebuilt.product, "q2-classic-baseq2");
        assert_eq!(rebuilt.map, "maps/base1.bsp");
        assert_eq!(rebuilt.movement, OptionsFamily::Q2);
        assert_eq!(rebuilt.character_model, "sarge");
        assert_eq!(rebuilt.rules, Some(MatchRules::Ctf));
        recipe.movement.provider = ProviderId::new("q9", "movement");
        let error = application_options_for_recipe(&test_options(), &catalog, &recipe).unwrap_err();
        assert!(matches!(error, ContentError::NoInputAdapter(_)));
    }

    #[test]
    fn options_rejects_unknown_character_model() {
        let catalog = stock_catalog();
        let geometry = scripted_reference("maps/base1.bsp", "q2-classic-baseq2");
        let plan = ResolvedMountPlan {
            id: create_mount_plan_id("test", "appearance").unwrap(),
            mounts: Vec::new(),
            default_order: Vec::new(),
            prefix_orders: Vec::new(),
        };
        let source = ProviderReference {
            provider: ProviderId::new("q2", "official"),
            content: ContentId("q2-classic-baseq2".to_string()),
        };
        let mut recipe = scripted_recipe(
            &catalog,
            GameFamily::Q2,
            "q2-classic-baseq2",
            geometry,
            plan,
            vec![typescript_execution(&source)],
        );
        recipe.character.appearance.provider = ProviderId::new("q2", "nomodel");
        let error = application_options_for_recipe(&test_options(), &catalog, &recipe).unwrap_err();
        assert!(matches!(error, ContentError::NoCharacterModel(_)));
    }

    #[test]
    fn q3_guest_assertion_matrix() {
        let catalog = stock_catalog();
        let geometry = scripted_reference("maps/q3dm1.bsp", "q3-baseq3");
        let plan = ResolvedMountPlan {
            id: create_mount_plan_id("test", "q3assert").unwrap(),
            mounts: Vec::new(),
            default_order: Vec::new(),
            prefix_orders: Vec::new(),
        };
        let owner = ProviderReference {
            provider: ProviderId::new("q3", "official"),
            content: ContentId("q3-baseq3".to_string()),
        };
        let execution = ExecutionModule::Qvm {
            owner: owner.clone(),
            artifact: geometry.clone(),
            role: ModuleRole::ServerGame,
            api: Q3ApiIdentity::Qagame(8),
        };
        let mut recipe = scripted_recipe(&catalog, GameFamily::Q3, "q3-baseq3", geometry, plan, vec![execution]);
        recipe.timing = vec![native_provider_timing(&owner, GameFamily::Q3, false)];
        assert_q3_guest_recipe(&recipe, &owner, &Q3ApiIdentity::Qagame(8)).unwrap();
        recipe.movement.provider = ProviderId::new("q2", "movement");
        let error = assert_q3_guest_recipe(&recipe, &owner, &Q3ApiIdentity::Qagame(8)).unwrap_err();
        assert!(matches!(error, ContentError::Q3GuestRecipe));
    }

    struct LoadFixture {
        _root: PathBuf,
        catalog: InstalledCatalog,
        recipe: ExecutableRecipe,
        options: ApplicationContentOptions,
    }

    fn load_fixture(name: &str) -> LoadFixture {
        let root = scratch_dir(name);
        let maps = root.join("maps");
        fs::create_dir_all(&maps).unwrap();
        fs::write(maps.join("test.bsp"), q1_test_bsp()).unwrap();
        fs::write(maps.join("test2.bsp"), q1_test_bsp()).unwrap();
        fs::write(maps.join("test.ent"), "{ \"classname\" \"worldspawn\" }\n").unwrap();
        let products = expected_products()
            .into_iter()
            .map(|expectation| {
                let loose_root = if expectation.id == "q1-classic-id1" {
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
        let catalog = InstalledCatalog::new(root.display().to_string(), products, Vec::new(), 0, None).unwrap();
        let mounts = catalog.mounts_for("q1-classic-id1").unwrap();
        let plan = ResolvedMountPlan {
            id: create_mount_plan_id("test", "geometry").unwrap(),
            default_order: mounts.iter().map(|mount| mount.identity().id.clone()).collect(),
            mounts,
            prefix_orders: Vec::new(),
        };
        let probe = open_mount_plan(&plan, OpenMountOptions::default()).unwrap();
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
        let recipe = scripted_recipe(
            &catalog,
            GameFamily::Q1,
            "q1-classic-id1",
            geometry,
            plan,
            vec![typescript_execution(&source)],
        );
        let options = ApplicationContentOptions::new(ApplicationOptions {
            product: "q1-classic-id1".to_string(),
            map: "maps/test.bsp".to_string(),
            ..ApplicationOptions::default()
        });
        LoadFixture {
            _root: root,
            catalog,
            recipe,
            options,
        }
    }

    #[test]
    fn load_restores_world_with_sidecars() {
        let fixture = load_fixture("load");
        let mut arena = WorldByteArena::default();
        let mut preparer = StubPreparer::default();
        let mut loaded = load_application_content(
            &fixture.options,
            Some(fixture.recipe.clone()),
            None,
            Some(fixture.catalog.clone()),
            None,
            &StubCompat,
            &mut StubQ3,
            &mut preparer,
            &mut arena,
        )
        .unwrap();
        assert_eq!(loaded.world.kind(), "q1-bsp");
        assert_eq!(loaded.world.entities(), "{ \"classname\" \"worldspawn\" }\n");
        assert_eq!(loaded.map_sidecars.len(), 2);
        assert_eq!(loaded.map_sidecars[0].path, "maps/test.ent");
        assert!(loaded.map_sidecars[0].resource.is_some());
        assert_eq!(loaded.map_sidecars[1].path, "maps/test.lit");
        assert!(loaded.map_sidecars[1].resource.is_none());
        assert_eq!(loaded.prepared_mods(), &["Gameplay".to_string()]);
        assert_eq!(preparer.mods, 1);
        assert_eq!(preparer.configured, 0);
        let traveled = resolve_application_travel(&loaded, "maps/test2.bsp").unwrap();
        assert_eq!(traveled.map.geometry.requested_path, "maps/test2.bsp");
        assert_eq!(traveled.resources.len(), 1);
        let scoped = loaded.for_content(&ContentId("q1-classic-id1".to_string())).unwrap();
        assert!(scoped.open("maps/test.bsp", |_| true).unwrap().is_some());
        assert_eq!(loaded.opened_mounts().len(), 2);
        let release = loaded.retain_main_mounts().unwrap();
        loaded.close();
        assert!(loaded.opened_mounts().is_empty());
        assert!(loaded.for_content(&ContentId("q1-classic-id1".to_string())).is_err());
        release();
        let _ = &loaded;
    }

    #[test]
    fn load_rejects_remote_mismatch() {
        let fixture = load_fixture("remote-mismatch");
        let options = ApplicationContentOptions {
            base: fixture.options.base.clone(),
            remote_content: Some(remote_content_selection(RemoteContentBase::Q2ClassicBaseq2, "baseq2").unwrap()),
            q3_product: None,
        };
        let mut arena = WorldByteArena::default();
        let mut preparer = StubPreparer::default();
        let error = load_application_content(
            &options,
            Some(fixture.recipe.clone()),
            None,
            Some(fixture.catalog.clone()),
            None,
            &StubCompat,
            &mut StubQ3,
            &mut preparer,
            &mut arena,
        )
        .unwrap_err();
        assert!(matches!(error, ContentError::RemoteProductMismatch));
    }

    #[test]
    fn load_rejects_unsupported_module() {
        let fixture = load_fixture("unsupported");
        let owner = ProviderReference {
            provider: ProviderId::new("q3", "official"),
            content: ContentId("q1-classic-id1".to_string()),
        };
        let native = ExecutionModule::Native {
            owner,
            artifact: scripted_reference("vm/qagame.qvm", "q1-classic-id1"),
            profile: NativeAbi::WindowsX86_64,
            role: ModuleRole::ServerGame,
            api: NativeModuleApi::Q3Qagame(8),
        };
        let mut recipe = fixture.recipe.clone();
        recipe.execution = vec![native];
        let mut arena = WorldByteArena::default();
        let mut preparer = StubPreparer::default();
        let error = load_application_content(
            &fixture.options,
            Some(recipe),
            None,
            Some(fixture.catalog.clone()),
            None,
            &StubCompat,
            &mut StubQ3,
            &mut preparer,
            &mut arena,
        )
        .unwrap_err();
        assert!(matches!(error, ContentError::UnsupportedModule(_)));
    }

    #[test]
    fn load_rejects_qvm_without_deathmatch() {
        let fixture = load_fixture("qvm-reject");
        let owner = ProviderReference {
            provider: ProviderId::new("q1", "official"),
            content: ContentId("q1-classic-id1".to_string()),
        };
        let qvm = ExecutionModule::Qvm {
            owner,
            artifact: scripted_reference("vm/qagame.qvm", "q1-classic-id1"),
            role: ModuleRole::ServerGame,
            api: Q3ApiIdentity::Qagame(8),
        };
        let mut recipe = fixture.recipe.clone();
        recipe.execution = vec![qvm];
        let mut arena = WorldByteArena::default();
        let mut preparer = StubPreparer::default();
        let error = load_application_content(
            &fixture.options,
            Some(recipe),
            None,
            Some(fixture.catalog.clone()),
            None,
            &StubCompat,
            &mut StubQ3,
            &mut preparer,
            &mut arena,
        )
        .unwrap_err();
        assert!(matches!(error, ContentError::QvmRejected));
    }

    #[test]
    fn configuration_content_from_recipe() {
        let fixture = load_fixture("config");
        let content = open_application_configuration_content(
            fixture.catalog.clone(),
            ApplicationConfigurationRequest::Recipe {
                recipe: fixture.recipe.clone(),
            },
            None,
            &mut StubQ3,
        )
        .unwrap();
        assert_eq!(content.selection.source.provider, ProviderId::new("q1", "official"));
        assert_eq!(content.selection.movement.provider, ProviderId::new("q1", "movement"));
        assert_eq!(content.selection.timing.len(), 1);
        assert!(content.q3_product.is_none());
        content.close();
    }

    #[test]
    fn remote_configuration_content_from_mounted() {
        let catalog = stock_catalog();
        let plan = ResolvedMountPlan {
            id: create_mount_plan_id("test", "remote-config").unwrap(),
            mounts: Vec::new(),
            default_order: Vec::new(),
            prefix_orders: Vec::new(),
        };
        let mounts = open_mount_plan(&plan, OpenMountOptions::default()).unwrap();
        let mounted = MountedApplicationContent {
            catalog,
            mounts,
            q3_product: None,
        };
        let content = remote_configuration_content(&test_options(), mounted).unwrap();
        assert_eq!(content.selection.source.provider, ProviderId::new("q2", "official"));
        content.close();
    }

    fn empty_pak() -> Vec<u8> {
        let mut header = Vec::from(*b"PACK");
        header.extend_from_slice(&12u32.to_le_bytes());
        header.extend_from_slice(&0u32.to_le_bytes());
        header
    }

    #[test]
    fn remote_content_opens_quakeworld_writes() {
        let root = scratch_dir("remote");
        let corpus = root.join("corpus");
        let user = root.join("user");
        let id1 = corpus.join("q1").join("id1");
        fs::create_dir_all(&id1).unwrap();
        fs::create_dir_all(corpus.join("q1").join("qw")).unwrap();
        fs::create_dir_all(&user).unwrap();
        fs::write(id1.join("pak0.pak"), empty_pak()).unwrap();
        fs::write(id1.join("pak1.pak"), empty_pak()).unwrap();
        let options = ApplicationContentOptions::new(ApplicationOptions {
            corpus_root: corpus.display().to_string(),
            user_content_root: Some(user.display().to_string()),
            ..ApplicationOptions::default()
        });
        let requested = remote_content_selection(RemoteContentBase::Q1Quakeworld, "qw").unwrap();
        let content = open_remote_content(&options, &requested, &|| {}, 0, &mut StubQ3).unwrap();
        assert_eq!(content.selection.directory, "qw");
        assert!(content.write_root.ends_with("qw"));
        assert!(Path::new(&content.write_root).is_dir());
        content.mounts.close();
        let _ = fs::remove_dir_all(&root);
    }

    fn q3_seed_plan() -> ResolvedMountPlan {
        ResolvedMountPlan {
            id: create_mount_plan_id("test", "q3bind").expect("plan id"),
            mounts: Vec::new(),
            default_order: Vec::new(),
            prefix_orders: Vec::new(),
        }
    }

    fn q3_base_options() -> ApplicationOptions {
        ApplicationOptions {
            product: "q3-baseq3".to_string(),
            ..ApplicationOptions::default()
        }
    }

    #[test]
    fn q3_client_content_drives_loaded_application_content() {
        use crate::bootstrap::network::q3_client_content::{Q3ClientContent, Q3ClientLoadedContent};

        let fixture = load_fixture("q3-client-bind");
        let mut arena = WorldByteArena::default();
        let mut preparer = StubPreparer::default();
        let loaded = load_application_content(
            &fixture.options,
            Some(fixture.recipe.clone()),
            None,
            Some(fixture.catalog.clone()),
            None,
            &StubCompat,
            &mut StubQ3,
            &mut preparer,
            &mut arena,
        )
        .expect("load");
        let mounts = loaded.catalog_mounts();
        assert!(std::ptr::eq(mounts.catalog, &loaded.catalog));
        assert_eq!(mounts.plan.id, loaded.mounts.plan.id);
        assert_eq!(loaded.mounted_content().len(), 1);

        let seed = stock_catalog();
        let plan = q3_seed_plan();
        let info = "\\sv_pure\\0\\fs_game\\";
        let mut content = Q3ClientContent::open(&q3_base_options(), info, 7, &seed, &plan, |policy| {
            assert!(policy.is_none());
            Ok(loaded)
        })
        .expect("open");
        assert!(!content.pure());
        assert!(content.content().is_some());
        assert!(content.matches(info, 7));
        let command = content.referenced_pure_command(3).expect("command");
        assert!(command.starts_with("cp "));
        content.close();
        assert!(content.content().is_none());
    }

    #[test]
    fn q3_client_content_open_loaded_runs_canonical_loader() {
        use crate::bootstrap::network::q3_client_content::{Q3ClientContent, Q3ClientContentError};

        let seed = stock_catalog();
        let plan = q3_seed_plan();
        let options = ApplicationContentOptions {
            base: q3_base_options(),
            remote_content: Some(remote_content_selection(RemoteContentBase::Q3Baseq3, "baseq3").expect("remote")),
            q3_product: None,
        };
        let mut arena = WorldByteArena::default();
        let mut preparer = StubPreparer::default();
        let result = Q3ClientContent::open_loaded(
            &options,
            "\\sv_pure\\0\\fs_game\\",
            7,
            &seed,
            &plan,
            None,
            &StubCompat,
            &mut StubQ3,
            &mut preparer,
            &mut arena,
        );
        let Err(error) = result else {
            panic!("loader runs");
        };
        assert!(matches!(
            error,
            Q3ClientContentError::Load(ContentError::RemoteProductMismatch)
        ));
    }
}
