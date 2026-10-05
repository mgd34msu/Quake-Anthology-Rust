//! Application weapon-behavior selection ported from Quake-Anthology-TS
//! `src/app/bootstrap/weapon-behavior-selection.ts`.
//!
//! Catalog discovery, mount plans, and behavior documents reuse
//! [`qa_content`]; QuakeC programs reuse [`qa_guest`]. Guest behavior
//! services, QuakeC resource preparation
//! ([`prepare_quake_c_resources`](super::simulation::quakec_source::prepare_quake_c_resources)),
//! rerelease guest preparation
//! ([`prepare_rerelease_guest`](super::simulation::rerelease_guest_source::prepare_rerelease_guest)),
//! content mounts, and application mod state
//! ([`application_mod_choices`](super::mod_selection::application_mod_choices),
//! [`apply_application_mods`](super::mod_selection::apply_application_mods))
//! arrive through [`WeaponBehaviorHost`], which carries the host-side roots.

use std::fmt::Display;
use std::path::PathBuf;

use qa_content::catalog::{
    discover_installed_content, discover_native_weapon_behaviors, discover_qc_weapon_behaviors,
    discover_qvm_weapon_behaviors, read_weapon_behavior_document, BehaviorMounts, CatalogError, CatalogProduct,
    DiscoverContentOptions, InstalledCatalog, MountedWeaponBehaviorDiscovery, NativeWeaponBehaviorService,
    QcWeaponProgramSnapshot, QvmWeaponBehaviorService, WeaponBehaviorCompatibility,
};
use qa_content::contract::{
    create_mount_plan_id, same_qvm_weapon_layout, same_weapon_behavior, ContentDigest, ContentId, ContractError,
    ExecutableRecipe, ExecutionModule, GameFamily, ModuleIdentity, ModuleRole, NativeWeaponBehaviorDeclaration,
    ProjectileRole, ProviderReference, QvmAbiProfile, QvmWeaponBehaviorFields, QvmWeaponBehaviorLayout,
    ResolvedMountPlan, ResolvedWeaponBehaviorSelection, WeaponBehaviorCallback, WeaponBehaviorComponent,
    WeaponBehaviorDefinition,
};
use qa_content::mounts::{open_mount_plan, MountError, MountedContent, OpenMountOptions};
use qa_core::cvar::CvarRegistry;
use qa_core::identity::ProviderId;
use qa_guest::qc::program::{load_qc_program, QcProgram};
use qa_guest::GuestError;
use thiserror::Error;

use crate::options::{ApplicationOptions, WeaponBehavior as OptionsWeaponBehavior};
use crate::settings::json::Json;

/// Weapon-behavior selection failure.
#[derive(Debug, Error)]
pub enum WeaponBehaviorSelectionError {
    /// Invalid request or mismatched declaration (donor error text).
    #[error("{0}")]
    Invalid(String),
    /// Catalog failure.
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    /// Mount failure.
    #[error(transparent)]
    Mount(#[from] MountError),
    /// Guest program failure.
    #[error(transparent)]
    Guest(#[from] GuestError),
    /// Contract failure.
    #[error(transparent)]
    Contract(#[from] ContractError),
    /// Injected host failure.
    #[error("weapon behavior host: {0}")]
    Host(String),
}

fn invalid(message: impl Into<String>) -> WeaponBehaviorSelectionError {
    WeaponBehaviorSelectionError::Invalid(message.into())
}

fn host_error(error: impl Display) -> WeaponBehaviorSelectionError {
    WeaponBehaviorSelectionError::Host(error.to_string())
}

/// Configured weapon-behavior request (`WeaponBehaviorRequest`):
/// `PRODUCT/DECLARED_ID`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponBehaviorRequest {
    /// Product name.
    pub product: String,
    /// Declared behavior ID.
    pub id: String,
}

/// One listed behavior choice (`ApplicationWeaponBehaviorChoice`).
#[derive(Debug, Clone, PartialEq)]
pub struct ApplicationWeaponBehaviorChoice {
    /// Choice identity (`PRODUCT/DECLARED_ID`).
    pub id: String,
    /// Display title.
    pub title: String,
    /// Unavailability reason, when the choice cannot run.
    pub unavailable: Option<String>,
    /// Resolved selection, when the choice can run.
    pub selection: Option<ResolvedWeaponBehaviorSelection>,
}

/// Prepared executable behavior (`PreparedWeaponBehavior`).
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum PreparedWeaponBehavior<Artifact, Profile, QuakeCResources, RereleaseGuest> {
    /// QVM behavior with its artifact, profile, and mounts.
    Qvm {
        /// Verified selection.
        selection: ResolvedWeaponBehaviorSelection,
        /// Bytecode artifact.
        artifact: Artifact,
        /// Weapon profile.
        profile: Profile,
        /// Content mounts backing the artifact.
        mounts: MountedContent,
    },
    /// QuakeC behavior with its program, resources, and mounts.
    Quakec {
        /// Verified selection.
        selection: ResolvedWeaponBehaviorSelection,
        /// Loaded program.
        program: QcProgram,
        /// Prepared resources.
        resources: QuakeCResources,
        /// Content mounts backing the artifact.
        mounts: MountedContent,
    },
    /// Rerelease native behavior with its guest, declaration, and mounts.
    RereleaseNative {
        /// Verified selection.
        selection: ResolvedWeaponBehaviorSelection,
        /// Prepared guest.
        prepared: RereleaseGuest,
        /// Retained declaration.
        declaration: NativeWeaponBehaviorDeclaration,
        /// Content mounts backing the artifact.
        mounts: MountedContent,
    },
}

/// Unported sibling operations behind weapon-behavior selection.
pub trait WeaponBehaviorHost<Artifact, Profile> {
    /// Host failure.
    type Error: Display;
    /// Prepared QuakeC resources (`PreparedQuakeCSource["resources"]`).
    type QuakeCResources;
    /// Prepared rerelease guest (`PreparedRereleaseGuest`).
    type RereleaseGuest;

    /// QVM behavior service (`qvm-weapon-behaviors.ts` bridge).
    fn qvm_service(&self) -> &dyn QvmWeaponBehaviorService<Artifact, Profile>;
    /// Native behavior service (`native-weapon-behaviors.ts` bridge).
    fn native_service(&self) -> &dyn NativeWeaponBehaviorService;
    /// Open content mounts for preparation (`forContent`).
    fn content_mounts(&mut self, content: &ContentId) -> Result<MountedContent, Self::Error>;
    /// Apply mod and behavior requests to a recipe (`applyApplicationMods`).
    fn apply_requests(
        &mut self,
        catalog: &InstalledCatalog,
        recipe: ExecutableRecipe,
        requests: &[WeaponBehaviorRequest],
    ) -> Result<ExecutableRecipe, Self::Error>;
    /// Snapshot a QuakeC program for behavior resolution.
    fn weapon_snapshot(&self, program: &QcProgram) -> QcWeaponProgramSnapshot;
    /// Prepare QuakeC resources (`prepareQuakeCResources`).
    fn prepare_quakec_resources(
        &mut self,
        program: &QcProgram,
        mounts: &MountedContent,
    ) -> Result<Self::QuakeCResources, Self::Error>;
    /// Prepare a rerelease native guest (`prepareRereleaseGuest` over
    /// `parsePe` bytes).
    fn prepare_rerelease_guest(
        &mut self,
        owner: &ProviderReference,
        artifact: &qa_content::contract::ResolvedResourceReference,
        image: &[u8],
        mounts: &MountedContent,
    ) -> Result<Self::RereleaseGuest, Self::Error>;
}

/// QVM artifact surface read by selection (`entry.artifact`).
pub trait QvmBehaviorArtifact {
    /// Artifact module identity.
    fn behavior_module_id(&self) -> ProviderId;
    /// Declared ABI profile, when the artifact pins one.
    fn behavior_abi_profile(&self) -> Option<QvmAbiProfile>;
}

/// QVM profile surface read by selection (`entry.profile`).
pub trait QvmBehaviorProfile {
    /// Behavior definition.
    fn behavior_definition(&self) -> &WeaponBehaviorDefinition;
    /// Entity stride.
    fn behavior_entity_stride(&self) -> u64;
    /// Level-time address.
    fn behavior_level_time(&self) -> u32;
    /// Allocate entry.
    fn behavior_allocate(&self) -> u32;
    /// Free entry.
    fn behavior_free(&self) -> u32;
    /// Behavior fields.
    fn behavior_fields(&self) -> QvmWeaponBehaviorFields;
}

/// Build a component layout from a QVM profile. The ported contract layout
/// carries no `fireAbi` field, so only the five carried offsets are mapped.
fn qvm_layout(profile: &impl QvmBehaviorProfile) -> QvmWeaponBehaviorLayout {
    QvmWeaponBehaviorLayout {
        entity_stride: profile.behavior_entity_stride(),
        level_time: profile.behavior_level_time(),
        allocate: profile.behavior_allocate(),
        free: profile.behavior_free(),
        fields: profile.behavior_fields(),
    }
}

/// JavaScript `RegExp` `\s` set (`WhiteSpace` plus `LineTerminator`).
fn is_js_whitespace(value: char) -> bool {
    matches!(
        value,
        '\u{0009}'..='\u{000d}'
            | '\u{0020}'
            | '\u{00a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
            | '\u{feff}'
    )
}

/// Read a `PRODUCT/DECLARED_ID` request (`readWeaponBehaviorRequest`).
pub fn read_weapon_behavior_request(value: &str) -> Result<WeaponBehaviorRequest, WeaponBehaviorSelectionError> {
    const MESSAGE: &str = "Weapon behavior must be PRODUCT/DECLARED_ID";
    let Some(slash) = value.find('/') else {
        return Err(invalid(MESSAGE));
    };
    if slash == 0
        || slash == value.len() - 1
        || value.chars().any(is_js_whitespace)
        || !value[slash + 1..].contains(':')
    {
        return Err(invalid(MESSAGE));
    }
    Ok(WeaponBehaviorRequest {
        product: value[..slash].to_owned(),
        id: value[slash + 1..].to_owned(),
    })
}

/// Apply the `qts_weaponBehavior` cvar when no behavior or mods are
/// configured (`configuredWeaponBehaviorOptions`). The ported options model
/// an always-present mod list, so a non-empty list counts as configured.
pub fn configured_weapon_behavior_options(
    options: ApplicationOptions,
    cvars: &CvarRegistry,
) -> Result<ApplicationOptions, WeaponBehaviorSelectionError> {
    if options.weapon_behavior.is_some() || !options.mods.is_empty() {
        return Ok(options);
    }
    let value = cvars
        .get("qts_weaponBehavior")
        .map(|cvar| cvar.value)
        .unwrap_or_default();
    if value.is_empty() {
        return Ok(options);
    }
    let request = read_weapon_behavior_request(&value)?;
    Ok(ApplicationOptions {
        weapon_behavior: Some(OptionsWeaponBehavior {
            product: request.product,
            id: request.id,
        }),
        ..options
    })
}

/// Module identity for a behavior artifact (`behaviorModule`).
pub(crate) fn behavior_module(content: &ContentId, path: &str, digest: &ContentDigest) -> ModuleIdentity {
    ModuleIdentity {
        id: ProviderId::new("weapon-behavior", content.as_str()),
        artifact_path: path.to_owned(),
        digest: digest.clone(),
        revision: digest.to_string(),
    }
}

/// Behavior provider identity (`weapon-behavior:PRODUCT`).
pub(crate) fn weapon_behavior_provider(product: &ContentId) -> ProviderId {
    ProviderId::new("weapon-behavior", product.as_str())
}

/// Mount-plan identity for a behavior namespace over a product
/// (`createMountPlanId(namespace, hex(product.id))`).
pub(crate) fn mount_plan_id_for_product(
    namespace: &str,
    product: &ContentId,
) -> Result<qa_content::contract::MountPlanId, ContractError> {
    let mut revision = String::with_capacity(product.as_str().len() * 2);
    for byte in product.as_str().bytes() {
        revision.push_str(&format!("{byte:02x}"));
    }
    create_mount_plan_id(namespace, &revision)
}

/// Default QuakeC program path for an edition.
pub(crate) fn default_progs_path(edition: &str) -> &'static str {
    if edition == "quakeworld" {
        "qwprogs.dat"
    } else {
        "progs.dat"
    }
}

/// Projectile role name.
pub(crate) fn projectile_role_name(role: ProjectileRole) -> &'static str {
    match role {
        ProjectileRole::Rocket => "rocket",
        ProjectileRole::Grenade => "grenade",
        ProjectileRole::Nail => "nail",
        ProjectileRole::Bolt => "bolt",
        ProjectileRole::Plasma => "plasma",
        ProjectileRole::Energy => "energy",
        ProjectileRole::Grapple => "grapple",
    }
}

/// Choice title (`TITLE — DEFINITION (ROLE)`).
fn behavior_choice_title(title: &str, definition: &WeaponBehaviorDefinition) -> String {
    format!(
        "{title} \u{2014} {} ({})",
        definition.title,
        projectile_role_name(definition.role)
    )
}

/// Single unavailable choice (`unavailable`).
fn unavailable_choice(
    product_id: &str,
    title: &str,
    reason: impl Into<String>,
) -> Vec<ApplicationWeaponBehaviorChoice> {
    vec![ApplicationWeaponBehaviorChoice {
        id: format!("{product_id}/unavailable"),
        title: title.to_owned(),
        unavailable: Some(reason.into()),
        selection: None,
    }]
}

/// List behavior choices from mounted content (`choicesFromMounts`).
#[allow(clippy::too_many_lines)]
fn choices_from_mounts<Artifact, Profile, H>(
    catalog: &InstalledCatalog,
    product_id: &str,
    mounts: &MountedContent,
    host: &mut H,
) -> Result<Vec<ApplicationWeaponBehaviorChoice>, WeaponBehaviorSelectionError>
where
    Artifact: QvmBehaviorArtifact,
    Profile: QvmBehaviorProfile,
    H: WeaponBehaviorHost<Artifact, Profile>,
{
    let product = catalog.require(product_id)?;
    let title = product.expectation.title.clone();
    let unavailable = |reason: String| unavailable_choice(product_id, &title, reason);
    if product.expectation.family == GameFamily::Q3 {
        let provider = weapon_behavior_provider(&product.id);
        let entries = discover_qvm_weapon_behaviors(mounts as &dyn BehaviorMounts, &provider, host.qvm_service())?;
        let Some(entries) = entries else {
            return Ok(unavailable(
                "No authored qvm-weapon-behaviors.json declaration with exact artifact and source layout".to_owned(),
            ));
        };
        if entries.is_empty() {
            return Ok(unavailable(
                "The source declares no QVM trajectory behaviors".to_owned(),
            ));
        }
        return Ok(entries
            .iter()
            .map(|entry| {
                let definition = entry.profile.behavior_definition();
                ApplicationWeaponBehaviorChoice {
                    id: format!("{product_id}/{}", definition.id),
                    title: behavior_choice_title(&title, definition),
                    unavailable: None,
                    selection: Some(ResolvedWeaponBehaviorSelection {
                        component: Some(WeaponBehaviorComponent::Qvm {
                            abi_profile: entry.artifact.behavior_abi_profile().unwrap_or(QvmAbiProfile::Modern),
                            layout: qvm_layout(&entry.profile),
                        }),
                        source: ProviderReference {
                            provider: entry.artifact.behavior_module_id(),
                            content: product.id.clone(),
                        },
                        artifact: entry.resource.clone(),
                        definition: definition.clone(),
                    }),
                }
            })
            .collect());
    }
    if product.expectation.family == GameFamily::Q2 && product.expectation.edition == "rerelease" {
        let provider = weapon_behavior_provider(&product.id);
        let entries =
            discover_native_weapon_behaviors(mounts as &dyn BehaviorMounts, &provider, host.native_service())?;
        let Some(entries) = entries else {
            return Ok(unavailable(
                "No authored native-weapon-behaviors.json declaration or matching built-in native profile".to_owned(),
            ));
        };
        if entries.is_empty() {
            return Ok(unavailable(
                "The source declares no native trajectory behaviors".to_owned(),
            ));
        }
        return Ok(entries
            .iter()
            .map(|entry| ApplicationWeaponBehaviorChoice {
                id: format!("{product_id}/{}", entry.definition.id),
                title: behavior_choice_title(&title, &entry.definition),
                unavailable: None,
                selection: Some(ResolvedWeaponBehaviorSelection {
                    component: Some(WeaponBehaviorComponent::RereleaseNative {
                        declaration: entry.declaration.clone(),
                    }),
                    source: ProviderReference {
                        provider: entry.definition.module.id.clone(),
                        content: product.id.clone(),
                    },
                    artifact: entry.resource.clone(),
                    definition: entry.definition.clone(),
                }),
            })
            .collect());
    }
    if product.expectation.family != GameFamily::Q1 {
        return Ok(unavailable(
            "This provider has no supported declared or artifact-qualified trajectory adapter".to_owned(),
        ));
    }
    let Some(descriptor) = mounts.open_behavior("weapon-behaviors.json")? else {
        return Ok(unavailable(
            "No authored weapon-behaviors.json trajectory declaration; use weapon-behavior inspect to inspect source callbacks"
                .to_owned(),
        ));
    };
    let document = read_weapon_behavior_document(&descriptor.bytes)?;
    let path = document
        .artifact_path
        .clone()
        .unwrap_or_else(|| default_progs_path(&product.expectation.edition).to_owned());
    let Some(artifact) = mounts.open_behavior(&path)? else {
        return Ok(unavailable(format!("Declared behavior requires mounted {path}")));
    };
    let program = load_qc_program(&artifact.bytes, None, "progs.dat")?;
    let module = behavior_module(&product.id, &path, artifact.content_digest());
    let snapshot = host.weapon_snapshot(&program);
    let discovered = discover_qc_weapon_behaviors(mounts as &dyn BehaviorMounts, &module, &snapshot)?;
    let declarations = match discovered {
        MountedWeaponBehaviorDiscovery::Declared { declarations } => declarations,
        MountedWeaponBehaviorDiscovery::Undeclared { reason, .. } => {
            return Ok(unavailable(reason));
        }
    };
    if declarations.is_empty() {
        return Ok(unavailable("The source declares no trajectory behaviors".to_owned()));
    }
    Ok(declarations
        .iter()
        .enumerate()
        .map(|(index, value)| match value {
            WeaponBehaviorCompatibility::Unsupported { reason } => ApplicationWeaponBehaviorChoice {
                id: format!("{product_id}/unsupported-{index}"),
                title: title.clone(),
                unavailable: Some(reason.clone()),
                selection: None,
            },
            WeaponBehaviorCompatibility::Supported { definition } => ApplicationWeaponBehaviorChoice {
                id: format!("{product_id}/{}", definition.id),
                title: behavior_choice_title(&title, definition),
                unavailable: None,
                selection: Some(ResolvedWeaponBehaviorSelection {
                    component: None,
                    source: ProviderReference {
                        provider: module.id.clone(),
                        content: product.id.clone(),
                    },
                    artifact: artifact.reference.clone(),
                    definition: definition.clone(),
                }),
            },
        })
        .collect())
}

/// Open a behavior mount plan with default options (the donor's default
/// `openPlan`).
pub fn open_behavior_mount_plan(plan: &ResolvedMountPlan) -> Result<MountedContent, WeaponBehaviorSelectionError> {
    Ok(open_mount_plan(plan, OpenMountOptions::default())?)
}

/// List behavior choices for a product
/// (`applicationWeaponBehaviorChoices`). The plan opener mirrors the donor's
/// injectable `openPlan`; pass [`open_behavior_mount_plan`] for the default.
pub fn application_weapon_behavior_choices<Artifact, Profile, H, O>(
    catalog: &InstalledCatalog,
    product_id: &str,
    host: &mut H,
    open_plan: O,
) -> Result<Vec<ApplicationWeaponBehaviorChoice>, WeaponBehaviorSelectionError>
where
    Artifact: QvmBehaviorArtifact,
    Profile: QvmBehaviorProfile,
    H: WeaponBehaviorHost<Artifact, Profile>,
    O: FnOnce(&ResolvedMountPlan) -> Result<MountedContent, WeaponBehaviorSelectionError>,
{
    let product = catalog.require(product_id)?;
    let mounts = catalog.mounts_for(product.id.as_str())?;
    let plan = ResolvedMountPlan {
        id: mount_plan_id_for_product("weapon-behavior", &product.id)?,
        default_order: mounts.iter().map(|mount| mount.identity().id.clone()).collect(),
        prefix_orders: Vec::new(),
        mounts,
    };
    let mounted = open_plan(&plan)?;
    choices_from_mounts(catalog, product_id, &mounted, host)
}

/// Whether any execution module is a native or bytecode server game, which
/// has no shared projectile behavior hook.
fn native_server_game_present<Artifact>(execution: &[ExecutionModule<Artifact>]) -> bool {
    execution.iter().any(|module| match module {
        ExecutionModule::Builtin { .. } => false,
        ExecutionModule::Quakec { .. } => true,
        ExecutionModule::Qvm { role, .. } | ExecutionModule::Native { role, .. } => *role == ModuleRole::ServerGame,
    })
}

/// Missing-behavior error for a request over listed choices.
fn missing_behavior_message(request: &WeaponBehaviorRequest, choices: &[ApplicationWeaponBehaviorChoice]) -> String {
    let reason = choices
        .iter()
        .find_map(|choice| choice.unavailable.clone())
        .unwrap_or_else(|| "No such declared compatible behavior".to_owned());
    format!("Weapon behavior {}/{}: {reason}", request.product, request.id)
}

/// Select a configured behavior into a recipe
/// (`selectApplicationWeaponBehavior`).
pub fn select_application_weapon_behavior<Artifact, Profile, H, O>(
    catalog: &InstalledCatalog,
    recipe: &ExecutableRecipe,
    request: &WeaponBehaviorRequest,
    host: &mut H,
    open_plan: O,
) -> Result<ExecutableRecipe, WeaponBehaviorSelectionError>
where
    Artifact: QvmBehaviorArtifact,
    Profile: QvmBehaviorProfile,
    H: WeaponBehaviorHost<Artifact, Profile>,
    O: FnOnce(&ResolvedMountPlan) -> Result<MountedContent, WeaponBehaviorSelectionError>,
{
    if native_server_game_present(&recipe.execution) {
        return Err(invalid(
            "Selected native or bytecode game has no shared projectile behavior hook; choose a TypeScript launcher provider",
        ));
    }
    let choices = application_weapon_behavior_choices(catalog, &request.product, host, open_plan)?;
    let selected = choices.iter().find(|choice| {
        choice
            .selection
            .as_ref()
            .is_some_and(|selection| selection.definition.id == request.id)
    });
    let Some(selected) = selected else {
        return Err(invalid(missing_behavior_message(request, &choices)));
    };
    let Some(selection) = selected.selection.clone() else {
        return Err(invalid(missing_behavior_message(request, &choices)));
    };
    let mut next = recipe.clone();
    next.weapon_behaviors = vec![selection];
    Ok(next)
}

/// Collect configured mod and behavior requests.
fn collect_configured_requests(options: &ApplicationOptions) -> Vec<WeaponBehaviorRequest> {
    let mut requests: Vec<WeaponBehaviorRequest> = options
        .mods
        .iter()
        .map(|selection| WeaponBehaviorRequest {
            product: selection.product.clone(),
            id: selection.id.clone(),
        })
        .collect();
    if let Some(behavior) = &options.weapon_behavior {
        requests.push(WeaponBehaviorRequest {
            product: behavior.product.clone(),
            id: behavior.id.clone(),
        });
    }
    requests
}

/// Whether any request names a product outside the catalog.
fn has_unknown_request_product(products: &[CatalogProduct], requests: &[WeaponBehaviorRequest]) -> bool {
    requests.iter().any(|request| {
        !products
            .iter()
            .any(|product| product.expectation.id == request.product || product.id.as_str() == request.product)
    })
}

/// Catalog plus recipe after applying configured requests.
#[derive(Debug, Clone)]
pub struct ConfiguredApplicationRecipe {
    /// Catalog, rediscovered with mods when a request names a new product.
    pub catalog: InstalledCatalog,
    /// Recipe with applied requests.
    pub recipe: ExecutableRecipe,
}

/// Apply configured mod and behavior requests to a recipe
/// (`prepareConfiguredApplicationRecipe`).
pub fn prepare_configured_application_recipe<Artifact, Profile, H>(
    catalog: &InstalledCatalog,
    options: &ApplicationOptions,
    recipe: &ExecutableRecipe,
    host: &mut H,
) -> Result<ConfiguredApplicationRecipe, WeaponBehaviorSelectionError>
where
    H: WeaponBehaviorHost<Artifact, Profile>,
{
    let requests = collect_configured_requests(options);
    if requests.is_empty() {
        return Ok(ConfiguredApplicationRecipe {
            catalog: catalog.clone(),
            recipe: recipe.clone(),
        });
    }
    let catalog = if has_unknown_request_product(&catalog.products, &requests) {
        discover_installed_content(&DiscoverContentOptions {
            corpus_root: PathBuf::from(&catalog.corpus_root),
            user_content_root: catalog.user_content_root.as_ref().map(PathBuf::from),
            products: None,
            generation: catalog.generation,
            discover_mods: true,
            remote_content: None,
        })?
    } else {
        catalog.clone()
    };
    let recipe = host
        .apply_requests(&catalog, recipe.clone(), &requests)
        .map_err(host_error)?;
    Ok(ConfiguredApplicationRecipe { catalog, recipe })
}

/// Component kind discriminant for equality checks.
fn component_kind(component: Option<&WeaponBehaviorComponent>) -> Option<u8> {
    match component {
        None => None,
        Some(WeaponBehaviorComponent::Qvm { .. }) => Some(0),
        Some(WeaponBehaviorComponent::RereleaseNative { .. }) => Some(1),
    }
}

/// Verify a mounted redeclaration against the stored selection.
fn verify_prepared_selection(
    current: Option<&ResolvedWeaponBehaviorSelection>,
    selection: &ResolvedWeaponBehaviorSelection,
) -> Result<(), WeaponBehaviorSelectionError> {
    let Some(current) = current else {
        return Err(invalid(
            "Selected weapon behavior differs from its mounted declaration or artifact",
        ));
    };
    if current.artifact.requested_path != selection.artifact.requested_path
        || current.artifact.identity != selection.artifact.identity
        || current.source.provider != selection.source.provider
        || !same_weapon_behavior(&current.definition, &selection.definition)
    {
        return Err(invalid(
            "Selected weapon behavior differs from its mounted declaration or artifact",
        ));
    }
    if component_kind(current.component.as_ref()) != component_kind(selection.component.as_ref()) {
        return Err(invalid(
            "Selected behavior component differs from its mounted declaration",
        ));
    }
    if let (
        Some(WeaponBehaviorComponent::Qvm {
            abi_profile: current_abi,
            layout: current_layout,
        }),
        Some(WeaponBehaviorComponent::Qvm { abi_profile, layout }),
    ) = (current.component.as_ref(), selection.component.as_ref())
    {
        if current_abi != abi_profile || !same_qvm_weapon_layout(current_layout, layout) {
            return Err(invalid(
                "Selected QVM behavior layout differs from its mounted declaration",
            ));
        }
    }
    if let (
        Some(WeaponBehaviorComponent::RereleaseNative {
            declaration: current_declaration,
        }),
        Some(WeaponBehaviorComponent::RereleaseNative { declaration }),
    ) = (current.component.as_ref(), selection.component.as_ref())
    {
        if !same_native_weapon_declaration(current_declaration, declaration) {
            return Err(invalid(
                "Selected native behavior profile differs from its mounted declaration",
            ));
        }
    }
    Ok(())
}

/// Prepare an executable behavior from a stored selection
/// (`prepareApplicationWeaponBehavior`).
pub fn prepare_application_weapon_behavior<Artifact, Profile, H>(
    catalog: &InstalledCatalog,
    selection: &ResolvedWeaponBehaviorSelection,
    host: &mut H,
) -> Result<
    PreparedWeaponBehavior<Artifact, Profile, H::QuakeCResources, H::RereleaseGuest>,
    WeaponBehaviorSelectionError,
>
where
    Artifact: QvmBehaviorArtifact,
    Profile: QvmBehaviorProfile,
    H: WeaponBehaviorHost<Artifact, Profile>,
{
    let product = catalog.product(selection.source.content.as_str())?;
    let mounts = host.content_mounts(&product.id).map_err(host_error)?;
    let choices = choices_from_mounts(catalog, &product.expectation.id, &mounts, host)?;
    let current = choices.iter().find_map(|choice| {
        choice
            .selection
            .as_ref()
            .filter(|current| current.definition.id == selection.definition.id)
    });
    verify_prepared_selection(current, selection)?;
    if matches!(selection.definition.fire, WeaponBehaviorCallback::Qvm { .. }) {
        let mut entries = discover_qvm_weapon_behaviors(
            &mounts as &dyn BehaviorMounts,
            &selection.source.provider,
            host.qvm_service(),
        )?
        .unwrap_or_default();
        let position = entries
            .iter()
            .position(|entry| host.qvm_service().qvm_weapon_profile_id(&entry.profile) == selection.definition.id);
        let Some(position) = position else {
            return Err(invalid("QVM behavior declaration changed during preparation"));
        };
        let entry = &entries[position];
        let definition = entry.profile.behavior_definition();
        let changed = match &selection.component {
            Some(WeaponBehaviorComponent::Qvm { abi_profile, layout }) => {
                !same_weapon_behavior(definition, &selection.definition)
                    || entry.artifact.behavior_abi_profile().unwrap_or(QvmAbiProfile::Modern) != *abi_profile
                    || !same_qvm_weapon_layout(&qvm_layout(&entry.profile), layout)
            }
            _ => true,
        };
        if changed {
            return Err(invalid("QVM behavior declaration changed during preparation"));
        }
        let entry = entries.remove(position);
        return Ok(PreparedWeaponBehavior::Qvm {
            selection: selection.clone(),
            artifact: entry.artifact,
            profile: entry.profile,
            mounts,
        });
    }
    let Some(artifact) = mounts.open_behavior(&selection.artifact.requested_path)? else {
        return Err(invalid("Selected weapon behavior program is missing"));
    };
    if matches!(selection.definition.fire, WeaponBehaviorCallback::NativeArtifact { .. }) {
        let Some(WeaponBehaviorComponent::RereleaseNative { declaration }) = selection.component.as_ref() else {
            return Err(invalid("Native weapon behavior is missing its retained declaration"));
        };
        let prepared = host
            .prepare_rerelease_guest(&selection.source, &artifact.reference, &artifact.bytes, &mounts)
            .map_err(host_error)?;
        return Ok(PreparedWeaponBehavior::RereleaseNative {
            selection: selection.clone(),
            prepared,
            declaration: declaration.clone(),
            mounts,
        });
    }
    if !matches!(selection.definition.fire, WeaponBehaviorCallback::Quakec { .. }) {
        return Err(invalid("Selected behavior has no executable preparation adapter"));
    }
    let program = load_qc_program(&artifact.bytes, None, "progs.dat")?;
    let resources = host.prepare_quakec_resources(&program, &mounts).map_err(host_error)?;
    Ok(PreparedWeaponBehavior::Quakec {
        selection: selection.clone(),
        program,
        resources,
        mounts,
    })
}

/// JSON number from an integer.
fn json_int(value: u64) -> Json {
    Json::Number(value as f64)
}

/// Serialize a native weapon registration layout.
fn registration_layout_json(layout: &qa_content::contract::NativeWeaponRegistrationLayout) -> Json {
    Json::Object(vec![
        ("byteLength".to_owned(), json_int(layout.byte_length)),
        ("name".to_owned(), json_int(layout.name)),
        ("tag".to_owned(), json_int(layout.tag)),
        ("callback".to_owned(), json_int(layout.callback)),
    ])
}

/// Serialize a native weapon entry.
fn native_entry_json(entry: &qa_content::contract::NativeWeaponEntry) -> Json {
    Json::Object(vec![
        ("rva".to_owned(), json_int(entry.rva)),
        (
            "registration".to_owned(),
            entry.registration.as_ref().map_or(Json::Null, |registration| {
                Json::Object(vec![
                    ("rva".to_owned(), json_int(registration.rva)),
                    ("name".to_owned(), Json::String(registration.name.clone())),
                    ("tag".to_owned(), json_int(registration.tag)),
                    ("layout".to_owned(), registration_layout_json(&registration.layout)),
                ])
            }),
        ),
    ])
}

/// Serialize a native weapon command.
fn native_command_json(command: &qa_content::contract::NativeWeaponCommand) -> Json {
    Json::Object(vec![
        (
            "arguments".to_owned(),
            Json::Array(command.arguments.iter().cloned().map(Json::String).collect()),
        ),
        ("tail".to_owned(), Json::String(command.tail.clone())),
    ])
}

/// Serialize a native weapon cvar.
fn native_cvar_json(cvar: &qa_content::contract::NativeWeaponCvar) -> Json {
    Json::Object(vec![
        ("name".to_owned(), Json::String(cvar.name.clone())),
        ("value".to_owned(), Json::String(cvar.value.clone())),
    ])
}

/// Serialize a native weapon behavior declaration to its document JSON.
/// Members follow the declaration interface order; validated constants
/// (`kind`, `abi`, `aspect`, signatures, storage) are re-emitted.
pub(crate) fn native_weapon_declaration_json(declaration: &NativeWeaponBehaviorDeclaration) -> Json {
    let entity = &declaration.entity;
    let client = &declaration.client;
    let equipped = &declaration.equipped_weapon;
    Json::Object(vec![
        ("version".to_owned(), Json::Number(1.0)),
        ("kind".to_owned(), Json::String("q2-api2023-trajectory".to_owned())),
        ("abi".to_owned(), Json::String("windows-x86-64".to_owned())),
        (
            "artifactPath".to_owned(),
            Json::String(declaration.artifact_path.clone()),
        ),
        (
            "artifactDigest".to_owned(),
            Json::String(declaration.artifact_digest.as_str().to_owned()),
        ),
        ("id".to_owned(), Json::String(declaration.id.clone())),
        ("title".to_owned(), Json::String(declaration.title.clone())),
        (
            "role".to_owned(),
            Json::String(projectile_role_name(declaration.role).to_owned()),
        ),
        ("aspect".to_owned(), Json::String("trajectory".to_owned())),
        (
            "entity".to_owned(),
            Json::Object(vec![
                ("byteLength".to_owned(), json_int(entity.byte_length)),
                ("origin".to_owned(), json_int(entity.origin)),
                ("angles".to_owned(), json_int(entity.angles)),
                ("velocity".to_owned(), json_int(entity.velocity)),
                ("client".to_owned(), json_int(entity.client)),
                ("owner".to_owned(), json_int(entity.owner)),
                ("viewHeight".to_owned(), json_int(entity.view_height)),
                ("generation".to_owned(), json_int(entity.generation)),
                ("nextThink".to_owned(), json_int(entity.next_think)),
                ("thinkCallback".to_owned(), json_int(entity.think_callback)),
                ("thinkRegistration".to_owned(), json_int(entity.think_registration)),
                ("touchCallback".to_owned(), json_int(entity.touch_callback)),
            ]),
        ),
        (
            "client".to_owned(),
            Json::Object(vec![
                ("byteLength".to_owned(), json_int(client.byte_length)),
                ("weapon".to_owned(), json_int(client.weapon)),
                ("viewAngles".to_owned(), json_int(client.view_angles)),
                ("forward".to_owned(), json_int(client.forward)),
            ]),
        ),
        (
            "equippedWeapon".to_owned(),
            Json::Object(vec![
                ("byteLength".to_owned(), json_int(equipped.byte_length)),
                ("callback".to_owned(), json_int(equipped.callback)),
                ("expected".to_owned(), native_entry_json(&equipped.expected)),
            ]),
        ),
        (
            "time".to_owned(),
            Json::Object(vec![
                ("storage".to_owned(), Json::String("int64-milliseconds".to_owned())),
                ("rva".to_owned(), json_int(declaration.time.rva)),
            ]),
        ),
        (
            "think".to_owned(),
            Json::Object(vec![
                ("signature".to_owned(), Json::String("entity-void".to_owned())),
                ("tag".to_owned(), json_int(declaration.think.tag)),
                (
                    "registration".to_owned(),
                    registration_layout_json(&declaration.think.registration),
                ),
            ]),
        ),
        (
            "allocate".to_owned(),
            Json::Object(vec![
                ("signature".to_owned(), Json::String("void-pointer".to_owned())),
                ("entry".to_owned(), native_entry_json(&declaration.allocate.entry)),
            ]),
        ),
        (
            "free".to_owned(),
            Json::Object(vec![
                ("signature".to_owned(), Json::String("entity-void".to_owned())),
                ("entry".to_owned(), native_entry_json(&declaration.free.entry)),
            ]),
        ),
        (
            "projectileTouch".to_owned(),
            native_entry_json(&declaration.projectile_touch),
        ),
        (
            "equip".to_owned(),
            Json::Object(vec![
                ("signature".to_owned(), Json::String("entity-void".to_owned())),
                (
                    "calls".to_owned(),
                    Json::Array(declaration.equip.calls.iter().map(native_entry_json).collect()),
                ),
            ]),
        ),
        (
            "launch".to_owned(),
            Json::Object(vec![
                ("signature".to_owned(), Json::String("entity-void".to_owned())),
                (
                    "calls".to_owned(),
                    Json::Array(declaration.launch.calls.iter().map(native_entry_json).collect()),
                ),
            ]),
        ),
        (
            "activateRva".to_owned(),
            declaration.activate_rva.map_or(Json::Null, json_int),
        ),
        ("fireRva".to_owned(), json_int(declaration.fire_rva)),
        (
            "initializationClasses".to_owned(),
            Json::Array(
                declaration
                    .initialization_classes
                    .iter()
                    .cloned()
                    .map(Json::String)
                    .collect(),
            ),
        ),
        (
            "equipment".to_owned(),
            Json::Array(declaration.equipment.iter().map(native_command_json).collect()),
        ),
        ("ammunition".to_owned(), native_command_json(&declaration.ammunition)),
        (
            "initialCvars".to_owned(),
            Json::Array(declaration.initial_cvars.iter().map(native_cvar_json).collect()),
        ),
        (
            "provisioningCvars".to_owned(),
            Json::Array(declaration.provisioning_cvars.iter().map(native_cvar_json).collect()),
        ),
    ])
}

/// Compare native weapon declarations by serialized form
/// (`sameNativeWeaponDeclaration`).
fn same_native_weapon_declaration(
    left: &NativeWeaponBehaviorDeclaration,
    right: &NativeWeaponBehaviorDeclaration,
) -> bool {
    native_weapon_declaration_json(left) == native_weapon_declaration_json(right)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::ModSelection;
    use qa_content::catalog::{ProductAvailability, ProductExpectation};
    use qa_content::contract::{
        ContentId, LooseMount, MountId, MountIdentity, MountPlanId, NativeAbi, NativeModuleApi, NativeWeaponAllocate,
        NativeWeaponCalls, NativeWeaponClient, NativeWeaponCommand, NativeWeaponCvar, NativeWeaponEntity,
        NativeWeaponEntry, NativeWeaponEquipped, NativeWeaponFree, NativeWeaponRegistrationLayout, NativeWeaponThink,
        NativeWeaponTime, Q3ApiIdentity, QuakeCApiIdentity, ResourceId, ResourceIdentity, ResourceProvenance,
        ResourceRequest, ResourceResolution, SourceModuleApi,
    };
    use qa_core::cmd::Dialect;
    use qa_core::cvar::flags;

    fn digest() -> ContentDigest {
        ContentDigest("sha256:ab".to_owned())
    }

    fn module() -> ModuleIdentity {
        behavior_module(&ContentId("q1:id1:progs:1".to_owned()), "progs.dat", &digest())
    }

    fn definition(id: &str) -> WeaponBehaviorDefinition {
        WeaponBehaviorDefinition {
            id: id.to_owned(),
            title: "Title".to_owned(),
            module: module(),
            role: ProjectileRole::Rocket,
            activate: None,
            fire: WeaponBehaviorCallback::Quakec {
                module: module(),
                function_index: 3,
            },
        }
    }

    fn reference(path: &str) -> qa_content::contract::ResolvedResourceReference {
        qa_content::contract::ResolvedResourceReference {
            id: ResourceId(format!("resource:seam:{path}")),
            requested_path: path.to_owned(),
            provenance: ResourceProvenance::Loose {
                mount: LooseMount {
                    identity: MountIdentity {
                        id: MountId("mount:seam:loose".to_owned()),
                        content: ContentId("q1:id1:progs:1".to_owned()),
                        generation: 0,
                    },
                    root_path: "/stub".to_owned(),
                },
                member_path: path.to_owned(),
            },
            identity: ResourceIdentity::parse("identity:0:0:8:0").unwrap(),
            byte_length: 8,
            resolution: ResourceResolution::DefaultOrder {
                plan: MountPlanId("mount-plan:seam:stub".to_owned()),
                rank: 0,
            },
        }
    }

    fn selection(definition_id: &str) -> ResolvedWeaponBehaviorSelection {
        ResolvedWeaponBehaviorSelection {
            component: None,
            source: ProviderReference {
                provider: ProviderId::new("weapon-behavior", "q1:id1:progs:1"),
                content: ContentId("q1:id1:progs:1".to_owned()),
            },
            artifact: reference("progs.dat"),
            definition: definition(definition_id),
        }
    }

    fn entry(rva: u64) -> NativeWeaponEntry {
        NativeWeaponEntry {
            rva,
            registration: None,
        }
    }

    fn declaration() -> NativeWeaponBehaviorDeclaration {
        let layout = NativeWeaponRegistrationLayout {
            byte_length: 1,
            name: 2,
            tag: 3,
            callback: 4,
        };
        NativeWeaponBehaviorDeclaration {
            version: 1,
            id: "native:rail".to_owned(),
            title: "Rail".to_owned(),
            role: ProjectileRole::Bolt,
            artifact_path: "game_x64.dll".to_owned(),
            artifact_digest: digest(),
            entity: NativeWeaponEntity {
                byte_length: 1,
                origin: 2,
                angles: 3,
                velocity: 4,
                client: 5,
                owner: 6,
                view_height: 7,
                generation: 8,
                next_think: 9,
                think_callback: 10,
                think_registration: 11,
                touch_callback: 12,
            },
            client: NativeWeaponClient {
                byte_length: 1,
                weapon: 2,
                view_angles: 3,
                forward: 4,
            },
            equipped_weapon: NativeWeaponEquipped {
                byte_length: 1,
                callback: 2,
                expected: entry(3),
            },
            time: NativeWeaponTime { rva: 4 },
            think: NativeWeaponThink {
                tag: 5,
                registration: layout,
            },
            allocate: NativeWeaponAllocate { entry: entry(6) },
            free: NativeWeaponFree { entry: entry(7) },
            projectile_touch: entry(8),
            equip: NativeWeaponCalls { calls: vec![entry(9)] },
            launch: NativeWeaponCalls { calls: vec![entry(10)] },
            activate_rva: None,
            fire_rva: 11,
            initialization_classes: vec!["G_Spawn".to_owned()],
            equipment: vec![NativeWeaponCommand {
                arguments: vec!["give".to_owned()],
                tail: "railgun".to_owned(),
            }],
            ammunition: NativeWeaponCommand {
                arguments: Vec::new(),
                tail: "slugs".to_owned(),
            },
            initial_cvars: vec![NativeWeaponCvar {
                name: "g_rail".to_owned(),
                value: "1".to_owned(),
            }],
            provisioning_cvars: Vec::new(),
        }
    }

    fn layout() -> QvmWeaponBehaviorLayout {
        QvmWeaponBehaviorLayout {
            entity_stride: 1,
            level_time: 2,
            allocate: 3,
            free: 4,
            fields: QvmWeaponBehaviorFields {
                inuse: 5,
                nextthink: 6,
                think: 7,
                health: 8,
            },
        }
    }

    fn product(family: GameFamily, edition: &str) -> CatalogProduct {
        CatalogProduct {
            id: ContentId("q1:id1:progs:1".to_owned()),
            expectation: ProductExpectation {
                id: "q1-classic-id1".to_owned(),
                family,
                edition: edition.to_owned(),
                campaign: "id1".to_owned(),
                title: "Quake".to_owned(),
                content_directory: "id1".to_owned(),
                base_product: None,
                required_content_archives: Vec::new(),
                required_programs: Vec::new(),
                map_witness: None,
                unresolved_reason: None,
            },
            availability: ProductAvailability::Installed,
            archives: Vec::new(),
            loose_root: None,
            user_content: None,
            maps: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn provider_ref() -> ProviderReference {
        ProviderReference {
            provider: ProviderId::new("engine", "typescript"),
            content: ContentId("q1:id1:progs:1".to_owned()),
        }
    }

    #[test]
    fn reads_product_and_declared_id() {
        let request = read_weapon_behavior_request("q1-classic-id1/qc:rail").unwrap();
        assert_eq!(request.product, "q1-classic-id1");
        assert_eq!(request.id, "qc:rail");
    }

    #[test]
    fn rejects_malformed_requests() {
        for value in [
            "no-slash",
            "/qc:rail",
            "product/",
            "pro duct/qc:rail",
            "product/qc\trail",
            "product/qcrail",
            "product/:",
        ] {
            if value == "product/:" {
                let request = read_weapon_behavior_request(value).unwrap();
                assert_eq!(request.id, ":");
                continue;
            }
            let error = read_weapon_behavior_request(value).unwrap_err();
            assert_eq!(
                error.to_string(),
                "Weapon behavior must be PRODUCT/DECLARED_ID",
                "{value}"
            );
        }
    }

    #[test]
    fn configured_options_keep_explicit_behavior() {
        let cvars = CvarRegistry::new(Dialect::Q1Netquake);
        let options = ApplicationOptions {
            weapon_behavior: Some(OptionsWeaponBehavior {
                product: "p".to_owned(),
                id: "qc:x".to_owned(),
            }),
            ..Default::default()
        };
        let next = configured_weapon_behavior_options(options.clone(), &cvars).unwrap();
        assert_eq!(next, options);
    }

    #[test]
    fn configured_options_keep_mods() {
        let cvars = CvarRegistry::new(Dialect::Q1Netquake);
        let options = ApplicationOptions {
            mods: vec![ModSelection {
                product: "p".to_owned(),
                id: "m".to_owned(),
            }],
            ..Default::default()
        };
        let next = configured_weapon_behavior_options(options.clone(), &cvars).unwrap();
        assert_eq!(next, options);
    }

    #[test]
    fn configured_options_read_cvar() {
        let mut cvars = CvarRegistry::new(Dialect::Q1Netquake);
        cvars.register("qts_weaponBehavior", "p/qc:rail", flags::NONE).unwrap();
        let next = configured_weapon_behavior_options(ApplicationOptions::default(), &cvars).unwrap();
        let behavior = next.weapon_behavior.unwrap();
        assert_eq!(behavior.product, "p");
        assert_eq!(behavior.id, "qc:rail");
    }

    #[test]
    fn configured_options_ignore_empty_cvar() {
        let cvars = CvarRegistry::new(Dialect::Q1Netquake);
        let next = configured_weapon_behavior_options(ApplicationOptions::default(), &cvars).unwrap();
        assert!(next.weapon_behavior.is_none());
    }

    #[test]
    fn behavior_module_pins_artifact() {
        let module = behavior_module(&ContentId("q1:x".to_owned()), "qwprogs.dat", &digest());
        assert_eq!(module.id.namespace, "weapon-behavior");
        assert_eq!(module.id.name, "q1:x");
        assert_eq!(module.artifact_path, "qwprogs.dat");
        assert_eq!(module.digest, digest());
        assert_eq!(module.revision, "sha256:ab");
    }

    #[test]
    fn plan_id_hex_encodes_product() {
        let id = mount_plan_id_for_product("weapon-behavior", &ContentId("q1:x".to_owned())).unwrap();
        assert_eq!(id.as_str(), "mount-plan:weapon-behavior:71313a78");
    }

    #[test]
    fn progs_path_follows_edition() {
        assert_eq!(default_progs_path("quakeworld"), "qwprogs.dat");
        assert_eq!(default_progs_path("classic"), "progs.dat");
        assert_eq!(default_progs_path("rerelease"), "progs.dat");
    }

    #[test]
    fn role_names_cover_contract_roles() {
        assert_eq!(projectile_role_name(ProjectileRole::Grapple), "grapple");
        assert_eq!(projectile_role_name(ProjectileRole::Plasma), "plasma");
    }

    #[test]
    fn choice_title_uses_em_dash_and_role() {
        assert_eq!(
            behavior_choice_title("Quake", &definition("qc:rail")),
            "Quake \u{2014} Title (rocket)"
        );
    }

    #[test]
    fn native_server_game_detection_matches_donor() {
        let typescript_server: ExecutionModule<ResourceRequest> = ExecutionModule::Builtin {
            owner: provider_ref(),
            implementation: ProviderId::new("engine", "typescript"),
            role: ModuleRole::ServerGame,
            api: SourceModuleApi::Q3Qagame(7),
        };
        assert!(!native_server_game_present(&[typescript_server]));
        let native_client = ExecutionModule::Native {
            owner: provider_ref(),
            artifact: ResourceRequest {
                content: ContentId("q2:r:game:1".to_owned()),
                path: "game.so".to_owned(),
            },
            profile: NativeAbi::LinuxX86_64,
            role: ModuleRole::ClientGame,
            api: NativeModuleApi::Q2ClassicGame,
        };
        assert!(!native_server_game_present(&[native_client]));
        let native_server = ExecutionModule::Native {
            owner: provider_ref(),
            artifact: ResourceRequest {
                content: ContentId("q2:r:game:1".to_owned()),
                path: "game.so".to_owned(),
            },
            profile: NativeAbi::WindowsX86_64,
            role: ModuleRole::ServerGame,
            api: NativeModuleApi::Q2RereleaseGame,
        };
        assert!(native_server_game_present(&[native_server]));
        let quakec = ExecutionModule::Quakec {
            owner: provider_ref(),
            artifact: ResourceRequest {
                content: ContentId("q1:x".to_owned()),
                path: "progs.dat".to_owned(),
            },
            api: QuakeCApiIdentity::Netquake,
        };
        assert!(native_server_game_present(&[quakec]));
        let qvm_ui = ExecutionModule::Qvm {
            owner: provider_ref(),
            artifact: ResourceRequest {
                content: ContentId("q3:x".to_owned()),
                path: "vm/ui.qvm".to_owned(),
            },
            role: ModuleRole::Ui,
            api: Q3ApiIdentity::Ui(6),
        };
        assert!(!native_server_game_present(&[qvm_ui]));
    }

    #[test]
    fn missing_behavior_prefers_first_unavailable_reason() {
        let request = WeaponBehaviorRequest {
            product: "p".to_owned(),
            id: "qc:missing".to_owned(),
        };
        assert_eq!(
            missing_behavior_message(&request, &[]),
            "Weapon behavior p/qc:missing: No such declared compatible behavior"
        );
        let choices = vec![ApplicationWeaponBehaviorChoice {
            id: "p/unavailable".to_owned(),
            title: "P".to_owned(),
            unavailable: Some("No declaration".to_owned()),
            selection: None,
        }];
        assert_eq!(
            missing_behavior_message(&request, &choices),
            "Weapon behavior p/qc:missing: No declaration"
        );
    }

    #[test]
    fn collect_requests_orders_mods_before_behavior() {
        let options = ApplicationOptions {
            weapon_behavior: Some(OptionsWeaponBehavior {
                product: "b".to_owned(),
                id: "qc:b".to_owned(),
            }),
            mods: vec![ModSelection {
                product: "m".to_owned(),
                id: "mod".to_owned(),
            }],
            ..Default::default()
        };
        let requests = collect_configured_requests(&options);
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].product, "m");
        assert_eq!(requests[1].product, "b");
        assert!(collect_configured_requests(&ApplicationOptions::default()).is_empty());
    }

    #[test]
    fn unknown_product_check_matches_either_identity() {
        let products = vec![product(GameFamily::Q1, "classic")];
        let known = vec![WeaponBehaviorRequest {
            product: "q1-classic-id1".to_owned(),
            id: "qc:x".to_owned(),
        }];
        assert!(!has_unknown_request_product(&products, &known));
        let known_content = vec![WeaponBehaviorRequest {
            product: "q1:id1:progs:1".to_owned(),
            id: "qc:x".to_owned(),
        }];
        assert!(!has_unknown_request_product(&products, &known_content));
        let unknown = vec![WeaponBehaviorRequest {
            product: "other".to_owned(),
            id: "qc:x".to_owned(),
        }];
        assert!(has_unknown_request_product(&products, &unknown));
    }

    #[test]
    fn verify_accepts_identical_selection() {
        let current = selection("qc:rail");
        verify_prepared_selection(Some(&current), &selection("qc:rail")).unwrap();
    }

    #[test]
    fn verify_rejects_moved_artifact() {
        let mut current = selection("qc:rail");
        current.artifact.requested_path = "other.dat".to_owned();
        let error = verify_prepared_selection(Some(&current), &selection("qc:rail")).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Selected weapon behavior differs from its mounted declaration or artifact"
        );
        let error = verify_prepared_selection(None, &selection("qc:rail")).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Selected weapon behavior differs from its mounted declaration or artifact"
        );
    }

    #[test]
    fn verify_rejects_component_mismatch() {
        let current = selection("qc:rail");
        let mut next = selection("qc:rail");
        next.component = Some(WeaponBehaviorComponent::Qvm {
            abi_profile: QvmAbiProfile::Modern,
            layout: layout(),
        });
        let error = verify_prepared_selection(Some(&current), &next).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Selected behavior component differs from its mounted declaration"
        );
    }

    #[test]
    fn verify_rejects_layout_mismatch() {
        let mut current = selection("qc:rail");
        current.component = Some(WeaponBehaviorComponent::Qvm {
            abi_profile: QvmAbiProfile::Modern,
            layout: layout(),
        });
        let mut next = selection("qc:rail");
        let mut other = layout();
        other.entity_stride = 9;
        next.component = Some(WeaponBehaviorComponent::Qvm {
            abi_profile: QvmAbiProfile::Modern,
            layout: other,
        });
        let error = verify_prepared_selection(Some(&current), &next).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Selected QVM behavior layout differs from its mounted declaration"
        );
    }

    #[test]
    fn verify_rejects_declaration_mismatch() {
        let mut current = selection("qc:rail");
        current.component = Some(WeaponBehaviorComponent::RereleaseNative {
            declaration: declaration(),
        });
        let mut next = selection("qc:rail");
        let mut other = declaration();
        other.fire_rva = 99;
        next.component = Some(WeaponBehaviorComponent::RereleaseNative { declaration: other });
        let error = verify_prepared_selection(Some(&current), &next).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Selected native behavior profile differs from its mounted declaration"
        );
        next.component = Some(WeaponBehaviorComponent::RereleaseNative {
            declaration: declaration(),
        });
        verify_prepared_selection(Some(&current), &next).unwrap();
    }

    #[test]
    fn declaration_json_round_trips_expected_members() {
        let json = native_weapon_declaration_json(&declaration());
        assert_eq!(
            json.get("kind"),
            Some(&Json::String("q2-api2023-trajectory".to_owned()))
        );
        assert_eq!(json.get("activateRva"), Some(&Json::Null));
        assert_eq!(json.get("fireRva"), Some(&Json::Number(11.0)));
        assert_eq!(
            json.get("entity").and_then(|entity| entity.get("nextThink")),
            Some(&Json::Number(9.0))
        );
        assert_eq!(
            json.get("ammunition").and_then(|value| value.get("tail")),
            Some(&Json::String("slugs".to_owned()))
        );
    }

    #[test]
    fn same_declaration_ignores_nothing() {
        assert!(same_native_weapon_declaration(&declaration(), &declaration()));
        let mut other = declaration();
        other.equipment.clear();
        assert!(!same_native_weapon_declaration(&declaration(), &other));
    }
}
