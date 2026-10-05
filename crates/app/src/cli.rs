//! Binary entry dispatch shared by `quake-anthology` and `qa-dedicated`.
//!
//! Donor provenance: `src/main.ts` (command branches, dedicated vs
//! windowed assembly, quit propagation). Signal handling stays with the
//! host: the loop checks [`Application::is_finished`](crate::application::Application::is_finished)
//! between frames, and the default SIGINT/SIGTERM disposition terminates a
//! headless run. The weapon-behavior branch runs
//! [`run_weapon_behavior_tool`](crate::bootstrap::weapon_behavior_tool::run_weapon_behavior_tool)
//! with a CLI host whose adapters delegate to the canonical homes:
//! QuakeC snapshots convert the loaded guest program; QVM artifacts resolve
//! through `qa_guest::qvm::artifacts` and profiles validate through
//! `qa_guest::qvm::weapon_behavior_profile` with provider images converted by
//! the simulation QVM weapon-behavior lane; native declarations read and
//! validate through `qa_compat::q2::rerelease::native_weapon_declaration`
//! with the built-in declaration from the Q2Eaks weapon profile; content
//! mounts open through the installed catalog plus a mount plan; recipe
//! requests apply through the mod-selection lane; QuakeC resources prepare
//! through the simulation QuakeC source; rerelease guests prepare through
//! the simulation rerelease guest source.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

use qa_client::render::NullRenderer;

use qa_compat::q2::compatibility::CompatibilityResource;
use qa_compat::q2::rerelease::native_weapon_declaration::{
    read_native_weapon_declaration as read_compat_native_declaration, validate_native_weapon_image, DeclValue,
    NativeWeaponBehaviorDeclaration as CompatNativeDeclaration, PeImageMirror, PeSectionMirror,
};
use qa_compat::q2::rerelease::q2eaks_weapon_profile::{
    q2eaks_weapon_declaration, WeaponBehaviorDeclaration as Q2eaksDeclaration, WeaponCommand as Q2eaksCommand,
    WeaponCvar as Q2eaksCvar, WeaponEntry as Q2eaksEntry, WeaponRegistration as Q2eaksRegistration,
    Q2EAKS_WEAPON_DIGEST,
};
use qa_content::catalog::{
    discover_installed_content, CatalogError, DiscoverContentOptions, InstalledCatalog, NativeWeaponBehaviorService,
    ProductAvailability, QcFunctionGlobal, QcWeaponFunction, QcWeaponOpcode, QcWeaponProgramSnapshot,
    QcWeaponStatement, QvmWeaponArtifactResolution, QvmWeaponBehaviorService,
};
use qa_content::contract::{
    ArchiveFormat, ContentDigest, ContentId, ExecutableRecipe, ModCallbackDeclaration, ModSelection, ModuleIdentity,
    NativeAbi, NativeCallAbi, NativeWeaponAllocate, NativeWeaponBehaviorDeclaration, NativeWeaponCalls,
    NativeWeaponClient, NativeWeaponCommand, NativeWeaponCvar, NativeWeaponEntity, NativeWeaponEntry,
    NativeWeaponEquipped, NativeWeaponFree, NativeWeaponRegistration, NativeWeaponRegistrationLayout,
    NativeWeaponThink, NativeWeaponTime, ProjectileRole, ProviderReference, QvmAbiProfile, QvmWeaponBehaviorFields,
    ResolvedMountPlan, ResolvedResourceReference, ResourceProvenance as ContractProvenance,
    ResourceResolution as ContractResolution, WeaponBehaviorCallback, WeaponBehaviorDefinition,
};
use qa_content::mods::ModsError;
use qa_content::mounts::{archive_digest_or_compute, open_mount_plan, MountError, MountedContent, OpenMountOptions};
use qa_content::q1::mods_callbacks::read_mod_callbacks;
use qa_content::value::{SaveJson, SaveReader};
use qa_core::identity::ProviderId;
use qa_guest::checkpoint::{GameApi, NativeCallAbi as CheckpointNativeCallAbi};
use qa_guest::core::contracts::{
    ContentDigest as GuestContentDigest, GuestAccess, ModuleIdentity as GuestModuleIdentity,
    NativeCallAbi as GuestNativeCallAbi,
};
use qa_guest::pe::format::parse_pe;
use qa_guest::qc::mod_provider::{QcApiKind, QcFunctionView, QcProgramView, QcValueType as ModQcValueType};
use qa_guest::qc::program::{QcFunction, QcOpcode, QcProgram, QcValueType as ProgramQcValueType, QuakeCApi};
use qa_guest::qc::weapon_behavior_profile::qc_weapon_behavior_capability_error;
use qa_guest::qvm::artifacts::{resolve_qvm_artifact as resolve_guest_qvm_artifact, ResolvedQvmArtifact};
use qa_guest::qvm::mod_provider::ProfileValue;
use qa_guest::qvm::syscalls::{QvmAbiProfile as GuestQvmAbiProfile, QvmRole as GuestQvmRole};
use qa_guest::qvm::weapon_behavior_profile::{
    read_qvm_weapon_profile as read_guest_qvm_profile, QvmWeaponProfile as GuestQvmWeaponProfile,
    WeaponBehaviorCallback as GuestWeaponBehaviorCallback, WeaponBehaviorDefinition as GuestWeaponBehaviorDefinition,
    WeaponBehaviorRole as GuestWeaponBehaviorRole,
};
use qa_world::save::shared::ProviderRef;

use crate::application::Application;
use crate::bootstrap::mod_selection::{
    application_mod_choices, apply_application_mods, ModSelectionError, WeaponBehaviorChoiceEntry,
};
use crate::bootstrap::simulation::q2_native_world::GuestSourceMounts;
use crate::bootstrap::simulation::quakec_source::{prepare_quake_c_resources, QuakeCSourceResource};
use crate::bootstrap::simulation::qvm_weapon_behavior::convert_artifact;
use crate::bootstrap::simulation::rerelease_guest_source::{
    prepare_rerelease_guest as prepare_sim_rerelease_guest, PreparedRereleaseGuest,
};
use crate::bootstrap::weapon_behavior_selection::{
    application_weapon_behavior_choices, mount_plan_id_for_product, native_weapon_declaration_json,
    QvmBehaviorArtifact, QvmBehaviorProfile, WeaponBehaviorHost, WeaponBehaviorRequest, WeaponBehaviorSelectionError,
};
use crate::bootstrap::weapon_behavior_tool::run_weapon_behavior_tool;
use crate::bootstrap::weapon_behavior_tool_options::{ProjectileRole as ToolProjectileRole, WeaponBehaviorToolCommand};
use crate::error::AppError;
use crate::options::{
    parse_application_command, ApplicationCommand, ProjectileRole as OptionsProjectileRole,
    WeaponBehaviorTool as ParsedWeaponBehaviorTool, HELP, WEAPON_BEHAVIOR_HELP,
};
use crate::persistence::recipe::{
    ContentMount as RecipeMount, ExecutionImplementation, MountIdentity as RecipeMountIdentity,
    ResolvedExecutionModule, ResolvedResourceReference as RecipeResourceReference,
    ResourceProvenance as RecipeProvenance, ResourceResolution as RecipeResolution,
};
use crate::settings::json::{stringify, Json};
use crate::startup::StartupConfig;

/// Run the application command line, returning the process exit code.
pub fn run(argv: &[String], stdout: &mut dyn Write, stderr: &mut dyn Write, version: &str) -> i32 {
    match run_inner(argv, stdout, version) {
        Ok(()) => 0,
        Err(error) => {
            let _ = writeln!(stderr, "quake-anthology: {error}");
            1
        }
    }
}

fn run_inner(argv: &[String], stdout: &mut dyn Write, version: &str) -> Result<(), AppError> {
    match parse_application_command(argv)? {
        ApplicationCommand::Help => {
            let _ = stdout.write_all(HELP.as_bytes());
            Ok(())
        }
        ApplicationCommand::Version => {
            let _ = writeln!(stdout, "Quake Anthology {version}");
            Ok(())
        }
        ApplicationCommand::WeaponBehavior { command } => {
            if matches!(command, ParsedWeaponBehaviorTool::Help) {
                let _ = stdout.write_all(WEAPON_BEHAVIOR_HELP.as_bytes());
                return Ok(());
            }
            run_weapon_behavior(&command, stdout)?;
            Ok(())
        }
        ApplicationCommand::ListContent { corpus_root } => {
            list_content(&corpus_root, stdout);
            Ok(())
        }
        ApplicationCommand::Run { options } => {
            if use_game_composition(&options) {
                run_windowed(&options, crate::bootstrap::startup::StartupEntry::Run, stdout)?;
                return Ok(());
            }
            run_dedicated(&options, stdout)?;
            Ok(())
        }
        ApplicationCommand::Menu { options } => {
            if use_game_composition(&options) {
                run_windowed(&options, crate::bootstrap::startup::StartupEntry::Menu, stdout)?;
                return Ok(());
            }
            run_dedicated(&options, stdout)?;
            Ok(())
        }
    }
}

/// Whether Run/Menu dispatches to the game composition.
///
/// Every non-dedicated run opens the game (a window, until quit or
/// `--frames`); only `--dedicated` serves headless. There is no third
/// mode: headless exists solely to bring up dedicated servers.
fn use_game_composition(options: &crate::options::ApplicationOptions) -> bool {
    !options.dedicated
}

/// Run the dedicated server: no window, no local seats, no player. This
/// is the only headless mode; everything else opens the game.
fn run_dedicated(options: &crate::options::ApplicationOptions, stdout: &mut dyn Write) -> Result<(), AppError> {
    if options.frame_timings {
        return Err(AppError::ConflictingOptions(
            "--frame-timings requires the game".to_string(),
        ));
    }
    let config = StartupConfig::from_options(options)?;
    let mut application = Application::open(&config, NullRenderer::new())?;
    let stats = application.run()?;
    let _ = writeln!(
        stdout,
        "Ran {} host frames, {} server ticks, {} entities ({} render frames)",
        stats.frames, stats.ticks, stats.entities, stats.render_frames
    );
    Ok(())
}

/// Run the windowed composition: native window, GL renderer, and startup
/// driver for `--windowed` runs.
fn run_windowed(
    options: &crate::options::ApplicationOptions,
    entry: crate::bootstrap::startup::StartupEntry,
    stdout: &mut dyn Write,
) -> Result<(), AppError> {
    let mut composed =
        crate::bootstrap::windowed::open_windowed_application(options, entry).map_err(AppError::Startup)?;
    let frames =
        crate::bootstrap::windowed::drive_windowed_application(&mut composed.app, &composed.quit, options.frame_limit)
            .map_err(|error| AppError::Startup(error.to_string()))?;
    let _ = writeln!(stdout, "Ran {frames} windowed frames");
    if let Some(report) = composed.app.backend().timing_report() {
        let _ = stdout.write_all(report.as_bytes());
    }
    Ok(())
}

/// Run the weapon-behavior tool (donor `main.ts` weapon-behavior branch).
fn run_weapon_behavior(command: &ParsedWeaponBehaviorTool, stdout: &mut dyn Write) -> Result<(), AppError> {
    let command = tool_command(command);
    let mut host = match &command {
        WeaponBehaviorToolCommand::Help => CliWeaponBehaviorHost::default(),
        WeaponBehaviorToolCommand::Inspect {
            corpus_root,
            user_content_root,
            ..
        }
        | WeaponBehaviorToolCommand::DeclareQvm {
            corpus_root,
            user_content_root,
            ..
        }
        | WeaponBehaviorToolCommand::DeclareNative {
            corpus_root,
            user_content_root,
            ..
        }
        | WeaponBehaviorToolCommand::Declare {
            corpus_root,
            user_content_root,
            ..
        } => CliWeaponBehaviorHost::with_roots(corpus_root.clone(), user_content_root.clone()),
    };
    let mut print = |text: String| {
        let _ = stdout.write_all(text.as_bytes());
    };
    run_weapon_behavior_tool(&command, &mut host, &mut print)?;
    Ok(())
}

/// Convert parsed CLI options into the tool command model.
fn tool_command(command: &ParsedWeaponBehaviorTool) -> WeaponBehaviorToolCommand {
    match command {
        ParsedWeaponBehaviorTool::Help => WeaponBehaviorToolCommand::Help,
        ParsedWeaponBehaviorTool::Inspect { content } => WeaponBehaviorToolCommand::Inspect {
            product: content.product.clone(),
            corpus_root: content.corpus_root.clone(),
            user_content_root: content.user_content_root.clone(),
            artifact: content.artifact.clone(),
        },
        ParsedWeaponBehaviorTool::DeclareQvm { content, profile } => WeaponBehaviorToolCommand::DeclareQvm {
            product: content.product.clone(),
            corpus_root: content.corpus_root.clone(),
            user_content_root: content.user_content_root.clone(),
            artifact: content.artifact.clone(),
            profile: profile.clone(),
        },
        ParsedWeaponBehaviorTool::DeclareNative { content, profile } => WeaponBehaviorToolCommand::DeclareNative {
            product: content.product.clone(),
            corpus_root: content.corpus_root.clone(),
            user_content_root: content.user_content_root.clone(),
            artifact: content.artifact.clone(),
            profile: profile.clone(),
        },
        ParsedWeaponBehaviorTool::Declare {
            content,
            id,
            title,
            role,
            fire,
            activate,
        } => WeaponBehaviorToolCommand::Declare {
            product: content.product.clone(),
            corpus_root: content.corpus_root.clone(),
            user_content_root: content.user_content_root.clone(),
            artifact: content.artifact.clone(),
            id: id.clone(),
            title: title.clone(),
            role: tool_role(*role),
            fire: fire.clone(),
            activate: activate.clone(),
        },
    }
}

/// Map a parsed projectile role onto the tool role.
fn tool_role(role: OptionsProjectileRole) -> ToolProjectileRole {
    match role {
        OptionsProjectileRole::Rocket => ToolProjectileRole::Rocket,
        OptionsProjectileRole::Grenade => ToolProjectileRole::Grenade,
        OptionsProjectileRole::Nail => ToolProjectileRole::Nail,
        OptionsProjectileRole::Bolt => ToolProjectileRole::Bolt,
        OptionsProjectileRole::Plasma => ToolProjectileRole::Plasma,
        OptionsProjectileRole::Energy => ToolProjectileRole::Energy,
        OptionsProjectileRole::Grapple => ToolProjectileRole::Grapple,
    }
}

/// Build a catalog failure from a canonical-home error.
fn catalog_invalid(error: impl ToString) -> CatalogError {
    CatalogError::Invalid(error.to_string())
}

/// CLI [`WeaponBehaviorHost`]. QuakeC snapshots convert the loaded guest
/// program; mounts open through the installed catalog plus a mount plan
/// (donor `catalog.mountsFor` + `openMountPlan` behind selection
/// `forContent`); recipe requests apply through
/// [`apply_application_mods`] over [`application_mod_choices`]; QuakeC
/// resources prepare through [`prepare_quake_c_resources`]; rerelease
/// guests prepare through [`prepare_sim_rerelease_guest`]. The content
/// roots feed lazily-discovered catalog state for content mounts.
#[derive(Default)]
struct CliWeaponBehaviorHost {
    qvm: CliQvmService,
    native: CliNativeService,
    corpus_root: String,
    user_content_root: String,
    catalog: Option<InstalledCatalog>,
}

impl CliWeaponBehaviorHost {
    /// Host resolving content mounts beneath the tool roots.
    fn with_roots(corpus_root: String, user_content_root: String) -> Self {
        Self {
            corpus_root,
            user_content_root,
            ..Self::default()
        }
    }

    /// Installed catalog for content mounts, discovered once (donor
    /// `discoverInstalledContent` with mod discovery, as the tool does).
    fn tool_catalog(&mut self) -> Result<&InstalledCatalog, CatalogError> {
        if self.catalog.is_none() {
            self.catalog = Some(discover_installed_content(&DiscoverContentOptions {
                corpus_root: PathBuf::from(&self.corpus_root),
                user_content_root: Some(PathBuf::from(&self.user_content_root)),
                products: None,
                generation: 0,
                discover_mods: true,
                remote_content: None,
            })?);
        }
        self.catalog
            .as_ref()
            .ok_or_else(|| catalog_invalid("CLI content catalog is unavailable"))
    }
}

/// Map a mod-selection failure onto the catalog error.
fn mod_selection_catalog(error: ModSelectionError) -> CatalogError {
    match error {
        ModSelectionError::Catalog(error) => error,
        ModSelectionError::Mount(error) => CatalogError::Mount(error),
        ModSelectionError::Contract(error) => CatalogError::Contract(error),
        ModSelectionError::Mod(message) | ModSelectionError::Mods(ModsError::Invalid(message)) => {
            catalog_invalid(message)
        }
        ModSelectionError::Mods(error) => catalog_invalid(error.to_string()),
    }
}

impl WeaponBehaviorHost<CliQvmArtifact, CliQvmProfile> for CliWeaponBehaviorHost {
    type Error = CatalogError;
    type QuakeCResources = HashMap<String, QuakeCSourceResource>;
    type RereleaseGuest = PreparedRereleaseGuest;

    fn qvm_service(&self) -> &dyn QvmWeaponBehaviorService<CliQvmArtifact, CliQvmProfile> {
        &self.qvm
    }

    fn native_service(&self) -> &dyn NativeWeaponBehaviorService {
        &self.native
    }

    fn content_mounts(&mut self, content: &ContentId) -> Result<MountedContent, Self::Error> {
        let mounts = self.tool_catalog()?.mounts_for(content.as_str())?;
        let plan = ResolvedMountPlan {
            id: mount_plan_id_for_product("weapon-behavior", content)?,
            default_order: mounts.iter().map(|mount| mount.identity().id.clone()).collect(),
            prefix_orders: Vec::new(),
            mounts,
        };
        Ok(open_mount_plan(&plan, OpenMountOptions::default())?)
    }

    fn apply_requests(
        &mut self,
        catalog: &InstalledCatalog,
        recipe: ExecutableRecipe,
        requests: &[WeaponBehaviorRequest],
    ) -> Result<ExecutableRecipe, Self::Error> {
        let mut entries: HashMap<String, Result<Vec<WeaponBehaviorChoiceEntry>, String>> = HashMap::new();
        for product in &catalog.products {
            if product.availability != ProductAvailability::Installed {
                continue;
            }
            let listed = application_weapon_behavior_choices(catalog, &product.expectation.id, self, |plan| {
                open_mount_plan(plan, OpenMountOptions::default()).map_err(WeaponBehaviorSelectionError::from)
            });
            entries.insert(
                product.expectation.id.clone(),
                listed
                    .map(|choices| {
                        choices
                            .into_iter()
                            .map(|choice| WeaponBehaviorChoiceEntry {
                                id: choice.id,
                                title: choice.title,
                                selection: choice.selection,
                                unavailable: choice.unavailable,
                            })
                            .collect()
                    })
                    .map_err(|error| error.to_string()),
            );
        }
        let weapon_choices = |product: &str| -> Result<Vec<WeaponBehaviorChoiceEntry>, ModSelectionError> {
            match entries.get(product) {
                None => Ok(Vec::new()),
                Some(Ok(choices)) => Ok(choices.clone()),
                Some(Err(reason)) => Err(ModSelectionError::Mod(reason.clone())),
            }
        };
        let choices = application_mod_choices(catalog, &weapon_choices, &quakec_callback_declaration)
            .map_err(mod_selection_catalog)?;
        let enabled: Vec<ModSelection> = requests
            .iter()
            .map(|request| ModSelection {
                product: request.product.clone(),
                id: request.id.clone(),
            })
            .collect();
        apply_application_mods(&recipe, &choices, &enabled).map_err(mod_selection_catalog)
    }

    fn weapon_snapshot(&self, program: &QcProgram) -> QcWeaponProgramSnapshot {
        weapon_snapshot(program)
    }

    fn prepare_quakec_resources(
        &mut self,
        program: &QcProgram,
        mounts: &MountedContent,
    ) -> Result<Self::QuakeCResources, Self::Error> {
        prepare_quake_c_resources(program, mounts, "").map_err(catalog_invalid)
    }

    fn prepare_rerelease_guest(
        &mut self,
        owner: &ProviderReference,
        artifact: &ResolvedResourceReference,
        image: &[u8],
        mounts: &MountedContent,
    ) -> Result<Self::RereleaseGuest, Self::Error> {
        let execution = ResolvedExecutionModule {
            owner: ProviderRef {
                provider: format!("{}:{}", owner.provider.namespace, owner.provider.name),
                content: owner.content.as_str().to_owned(),
            },
            role: "server-game".to_owned(),
            api: GameApi::Q2RereleaseGame,
            implementation: ExecutionImplementation::Native {
                artifact: recipe_resource(artifact).map_err(catalog_invalid)?,
                profile: CheckpointNativeCallAbi::WindowsX8664,
            },
        };
        let mounts = ImageGuestMounts {
            inner: mounts,
            path: artifact.requested_path.as_str(),
            bytes: image,
        };
        prepare_sim_rerelease_guest(&execution, &mounts).map_err(catalog_invalid)
    }
}

/// Guest source mounts serving the caller-opened image bytes for the
/// selected artifact while delegating every other read (including the
/// compatibility document) to the content mounts. The guest home
/// validates the exact bytes the selection opened against the artifact
/// digest instead of re-reading them.
struct ImageGuestMounts<'a> {
    inner: &'a MountedContent,
    path: &'a str,
    bytes: &'a [u8],
}

impl GuestSourceMounts for ImageGuestMounts<'_> {
    fn read_artifact(&self, path: &str) -> Result<Vec<u8>, qa_content::mounts::MountError> {
        if path == self.path {
            return Ok(self.bytes.to_vec());
        }
        self.inner.read_artifact(path)
    }

    fn open_compat_doc(&self) -> Option<CompatibilityResource> {
        self.inner.open_compat_doc()
    }
}

/// CLI QVM behavior service: artifacts resolve through
/// [`resolve_guest_qvm_artifact`] (donor `compat/qvm/artifacts.ts`) and
/// profiles validate through [`read_guest_qvm_profile`] (donor
/// `compat/qvm/weapon-behavior-profile.ts`), with the provider image
/// converted by the simulation QVM weapon-behavior lane.
#[derive(Default)]
struct CliQvmService;

/// CLI QVM artifact: the resolved guest artifact.
#[derive(Debug)]
struct CliQvmArtifact {
    artifact: ResolvedQvmArtifact,
}

/// CLI QVM profile: the validated guest profile plus its contract
/// definition for the selection surface.
#[derive(Debug)]
struct CliQvmProfile {
    profile: GuestQvmWeaponProfile,
    definition: WeaponBehaviorDefinition,
}

impl QvmWeaponBehaviorService<CliQvmArtifact, CliQvmProfile> for CliQvmService {
    fn resolve_qvm_artifact(
        &self,
        abi_profile: QvmAbiProfile,
        bytes: &[u8],
        module: &ModuleIdentity,
    ) -> Result<QvmWeaponArtifactResolution<CliQvmArtifact>, CatalogError> {
        let resolved = resolve_guest_qvm_artifact(
            &guest_module(module)?,
            GuestQvmRole::Qagame,
            bytes,
            Vec::new(),
            guest_abi_profile(abi_profile),
        )
        .map_err(catalog_invalid)?;
        if matches!(resolved, ResolvedQvmArtifact::Bytecode { .. }) {
            Ok(QvmWeaponArtifactResolution::Bytecode(CliQvmArtifact {
                artifact: resolved,
            }))
        } else {
            Ok(QvmWeaponArtifactResolution::Other)
        }
    }

    fn read_qvm_weapon_profile(
        &self,
        declaration: &SaveJson,
        artifact: &CliQvmArtifact,
    ) -> Result<CliQvmProfile, CatalogError> {
        let ResolvedQvmArtifact::Bytecode { module, .. } = &artifact.artifact else {
            return Err(catalog_invalid("QVM weapon behavior requires actual bytecode"));
        };
        let (_, provider) = convert_artifact(&artifact.artifact).map_err(catalog_invalid)?;
        let profile = read_guest_qvm_profile(&profile_value(declaration)?, &provider).map_err(catalog_invalid)?;
        let definition = guest_definition_to_contract(&profile.definition, module)?;
        Ok(CliQvmProfile { profile, definition })
    }

    fn qvm_weapon_profile_id(&self, profile: &CliQvmProfile) -> String {
        profile.definition.id.clone()
    }
}

impl QvmBehaviorArtifact for CliQvmArtifact {
    fn behavior_module_id(&self) -> ProviderId {
        match &self.artifact {
            ResolvedQvmArtifact::Bytecode { module, .. } | ResolvedQvmArtifact::TypeScript { module, .. } => {
                module.id.clone()
            }
        }
    }

    fn behavior_abi_profile(&self) -> Option<QvmAbiProfile> {
        match &self.artifact {
            ResolvedQvmArtifact::Bytecode { abi_profile, .. } => Some(contract_abi_profile(*abi_profile)),
            ResolvedQvmArtifact::TypeScript { .. } => None,
        }
    }
}

impl QvmBehaviorProfile for CliQvmProfile {
    fn behavior_definition(&self) -> &WeaponBehaviorDefinition {
        &self.definition
    }

    fn behavior_entity_stride(&self) -> u64 {
        u64::try_from(self.profile.layout.entity_stride).unwrap_or(u64::MAX)
    }

    fn behavior_level_time(&self) -> u32 {
        u32::try_from(self.profile.layout.level_time).unwrap_or(u32::MAX)
    }

    fn behavior_allocate(&self) -> u32 {
        u32::try_from(self.profile.layout.allocate).unwrap_or(u32::MAX)
    }

    fn behavior_free(&self) -> u32 {
        u32::try_from(self.profile.layout.free).unwrap_or(u32::MAX)
    }

    fn behavior_fields(&self) -> QvmWeaponBehaviorFields {
        let fields = self.profile.layout.fields;
        QvmWeaponBehaviorFields {
            inuse: u32::try_from(fields.inuse).unwrap_or(u32::MAX),
            nextthink: u32::try_from(fields.nextthink).unwrap_or(u32::MAX),
            think: u32::try_from(fields.think).unwrap_or(u32::MAX),
            health: u32::try_from(fields.health).unwrap_or(u32::MAX),
        }
    }
}

/// Contract module identity as a guest module identity.
fn guest_module(module: &ModuleIdentity) -> Result<GuestModuleIdentity, CatalogError> {
    let (algorithm, value) = module
        .digest
        .as_str()
        .split_once(':')
        .ok_or_else(|| catalog_invalid("QVM module digest is not an algorithm:value identity"))?;
    Ok(GuestModuleIdentity::new(
        module.id.clone(),
        &module.artifact_path,
        GuestContentDigest::new(algorithm, value),
        &module.revision,
    ))
}

/// Contract module identity from a guest module identity.
fn contract_module(module: &GuestModuleIdentity) -> ModuleIdentity {
    ModuleIdentity {
        id: module.id.clone(),
        artifact_path: module.artifact_path.clone(),
        digest: ContentDigest(format!("{}:{}", module.digest.algorithm, module.digest.value)),
        revision: module.revision.clone(),
    }
}

/// Guest ABI profile for a contract ABI profile.
fn guest_abi_profile(profile: QvmAbiProfile) -> GuestQvmAbiProfile {
    match profile {
        QvmAbiProfile::Modern => GuestQvmAbiProfile::Modern,
        QvmAbiProfile::Legacy116n => GuestQvmAbiProfile::Legacy116n,
    }
}

/// Contract ABI profile for a guest ABI profile.
fn contract_abi_profile(profile: GuestQvmAbiProfile) -> QvmAbiProfile {
    match profile {
        GuestQvmAbiProfile::Modern => QvmAbiProfile::Modern,
        GuestQvmAbiProfile::Legacy116n => QvmAbiProfile::Legacy116n,
    }
}

/// Contract projectile role for a guest weapon role.
fn contract_projectile_role(role: GuestWeaponBehaviorRole) -> ProjectileRole {
    match role {
        GuestWeaponBehaviorRole::Rocket => ProjectileRole::Rocket,
        GuestWeaponBehaviorRole::Grenade => ProjectileRole::Grenade,
        GuestWeaponBehaviorRole::Nail => ProjectileRole::Nail,
        GuestWeaponBehaviorRole::Bolt => ProjectileRole::Bolt,
        GuestWeaponBehaviorRole::Plasma => ProjectileRole::Plasma,
        GuestWeaponBehaviorRole::Energy => ProjectileRole::Energy,
        GuestWeaponBehaviorRole::Grapple => ProjectileRole::Grapple,
    }
}

/// Contract QVM callback for a validated guest profile callback. The
/// reader only produces QVM callbacks; any other shape is rejected.
fn contract_qvm_callback(
    callback: &GuestWeaponBehaviorCallback,
    module: &ModuleIdentity,
) -> Result<WeaponBehaviorCallback, CatalogError> {
    match callback {
        GuestWeaponBehaviorCallback::Qvm { instruction_index, .. } => {
            let index = u32::try_from(*instruction_index)
                .map_err(|_| catalog_invalid("QVM behavior callback index exceeds u32"))?;
            Ok(WeaponBehaviorCallback::Qvm {
                module: module.clone(),
                instruction_index: index,
            })
        }
        GuestWeaponBehaviorCallback::QuakeC { .. } | GuestWeaponBehaviorCallback::NativeArtifact { .. } => {
            Err(catalog_invalid("QVM weapon profile carries a non-QVM callback"))
        }
    }
}

/// Contract behavior definition for a validated guest profile. The
/// definition module is the artifact module the reader bound.
fn guest_definition_to_contract(
    definition: &GuestWeaponBehaviorDefinition,
    module: &GuestModuleIdentity,
) -> Result<WeaponBehaviorDefinition, CatalogError> {
    let module = contract_module(module);
    Ok(WeaponBehaviorDefinition {
        id: definition.id.clone(),
        title: definition.title.clone(),
        module: module.clone(),
        role: contract_projectile_role(definition.role),
        activate: definition
            .activate
            .as_ref()
            .map(|callback| contract_qvm_callback(callback, &module))
            .transpose()?,
        fire: contract_qvm_callback(&definition.fire, &module)?,
    })
}

/// Guest profile value for a content declaration value.
fn profile_value(value: &SaveJson) -> Result<ProfileValue, CatalogError> {
    match value {
        SaveJson::Null => Ok(ProfileValue::Null),
        SaveJson::Bool(value) => Ok(ProfileValue::Bool(*value)),
        SaveJson::Number(value) => {
            #[allow(clippy::cast_precision_loss)]
            let in_range = *value >= i64::MIN as f64 && *value <= i64::MAX as f64;
            if value.is_finite() && value.fract() == 0.0 && in_range {
                #[allow(clippy::cast_possible_truncation)]
                Ok(ProfileValue::Int(*value as i64))
            } else if value.is_finite() {
                Ok(ProfileValue::Float(*value))
            } else {
                Err(catalog_invalid("QVM weapon declaration number is not finite"))
            }
        }
        SaveJson::BigInt(value) => i64::try_from(*value)
            .map(ProfileValue::Int)
            .map_err(|_| catalog_invalid("QVM weapon declaration integer exceeds i64")),
        SaveJson::Bytes(value) => Ok(ProfileValue::Bytes(value.clone())),
        SaveJson::String(value) => Ok(ProfileValue::Str(value.clone())),
        SaveJson::Array(items) => items
            .iter()
            .map(profile_value)
            .collect::<Result<Vec<_>, _>>()
            .map(ProfileValue::Array),
        SaveJson::Object(members) => members
            .iter()
            .map(|(name, member)| profile_value(member).map(|value| (name.clone(), value)))
            .collect::<Result<Vec<_>, _>>()
            .map(ProfileValue::Record),
    }
}

/// CLI native behavior service: declarations read through
/// [`read_compat_native_declaration`] (donor
/// `compat/q2/rerelease/native-weapon-declaration.ts`), images validate
/// through [`validate_native_weapon_image`], definitions derive per donor
/// `rereleaseWeaponDefinition`, and the built-in declaration comes from
/// [`q2eaks_weapon_declaration`] (donor
/// `compat/q2/rerelease/q2eaks-weapon-profile.ts`).
#[derive(Default)]
struct CliNativeService;

impl NativeWeaponBehaviorService for CliNativeService {
    fn read_native_weapon_declaration(
        &self,
        value: &SaveJson,
    ) -> Result<NativeWeaponBehaviorDeclaration, CatalogError> {
        let parsed = read_compat_native_declaration(&declaration_value(value)?, None).map_err(catalog_invalid)?;
        compat_declaration_to_contract(&parsed)
    }

    fn native_weapon_definition(
        &self,
        declaration: &NativeWeaponBehaviorDeclaration,
        module: &ModuleIdentity,
        image_bytes: &[u8],
    ) -> Result<Option<WeaponBehaviorDefinition>, CatalogError> {
        let serialized = native_weapon_declaration_json(declaration);
        let parsed = read_compat_native_declaration(
            &json_declaration_value(&serialized)?,
            Some((module.digest.as_str(), module.artifact_path.as_str())),
        )
        .map_err(catalog_invalid)?;
        let image = parse_pe(image_bytes).map_err(catalog_invalid)?;
        validate_native_weapon_image(&parsed, &pe_image_mirror(&image)).map_err(catalog_invalid)?;
        Ok(Some(native_weapon_definition(module, declaration)))
    }

    fn builtin_rerelease_weapon_declaration(
        &self,
        module: &ModuleIdentity,
    ) -> Result<Option<NativeWeaponBehaviorDeclaration>, CatalogError> {
        if module.digest.as_str() != Q2EAKS_WEAPON_DIGEST {
            return Ok(None);
        }
        let builtin = q2eaks_weapon_declaration(&module.artifact_path);
        let parsed = read_compat_native_declaration(
            &q2eaks_declaration_value(&builtin)?,
            Some((module.digest.as_str(), module.artifact_path.as_str())),
        )
        .map_err(catalog_invalid)?;
        compat_declaration_to_contract(&parsed).map(Some)
    }
}

/// Executable definition for a validated native declaration (donor
/// `rereleaseWeaponDefinition`): identity, title, role, and native
/// fire/activate callbacks over the rerelease ABI.
fn native_weapon_definition(
    module: &ModuleIdentity,
    declaration: &NativeWeaponBehaviorDeclaration,
) -> WeaponBehaviorDefinition {
    let abi = NativeCallAbi::Native(NativeAbi::WindowsX86_64);
    WeaponBehaviorDefinition {
        id: declaration.id.clone(),
        title: declaration.title.clone(),
        module: module.clone(),
        role: declaration.role,
        activate: declaration
            .activate_rva
            .map(|offset| WeaponBehaviorCallback::NativeArtifact {
                module: module.clone(),
                image_offset: offset,
                abi,
            }),
        fire: WeaponBehaviorCallback::NativeArtifact {
            module: module.clone(),
            image_offset: declaration.fire_rva,
            abi,
        },
    }
}

/// Compat declaration value for a content declaration value.
fn declaration_value(value: &SaveJson) -> Result<DeclValue, CatalogError> {
    match value {
        SaveJson::Null => Ok(DeclValue::Null),
        SaveJson::Bool(value) => Ok(DeclValue::Bool(*value)),
        SaveJson::Number(value) => {
            if value.is_finite() && value.fract() == 0.0 {
                #[allow(clippy::cast_possible_truncation)]
                Ok(DeclValue::Int(*value as i64))
            } else {
                Err(catalog_invalid("Native weapon declaration needs integer numbers"))
            }
        }
        SaveJson::BigInt(value) => i64::try_from(*value)
            .map(DeclValue::Int)
            .map_err(|_| catalog_invalid("Native weapon declaration integer exceeds i64")),
        SaveJson::Bytes(_) => Err(catalog_invalid("Native weapon declaration has no byte fields")),
        SaveJson::String(value) => Ok(DeclValue::Str(value.clone())),
        SaveJson::Array(items) => items
            .iter()
            .map(declaration_value)
            .collect::<Result<Vec<_>, _>>()
            .map(DeclValue::List),
        SaveJson::Object(members) => {
            let mut map = HashMap::with_capacity(members.len());
            for (name, member) in members {
                map.insert(name.clone(), declaration_value(member)?);
            }
            Ok(DeclValue::Map(map))
        }
    }
}

/// Compat declaration value for a serialized declaration document.
fn json_declaration_value(value: &Json) -> Result<DeclValue, CatalogError> {
    match value {
        Json::Null => Ok(DeclValue::Null),
        Json::Bool(value) => Ok(DeclValue::Bool(*value)),
        Json::Number(value) => {
            if value.is_finite() && value.fract() == 0.0 {
                #[allow(clippy::cast_possible_truncation)]
                Ok(DeclValue::Int(*value as i64))
            } else {
                Err(catalog_invalid("Native weapon declaration needs integer numbers"))
            }
        }
        Json::String(value) => Ok(DeclValue::Str(value.clone())),
        Json::Array(items) => items
            .iter()
            .map(json_declaration_value)
            .collect::<Result<Vec<_>, _>>()
            .map(DeclValue::List),
        Json::Object(members) => {
            let mut map = HashMap::with_capacity(members.len());
            for (name, member) in members {
                map.insert(name.clone(), json_declaration_value(member)?);
            }
            Ok(DeclValue::Map(map))
        }
    }
}

/// Compat declaration value for an integer field.
fn decl_int(value: u64) -> Result<DeclValue, CatalogError> {
    i64::try_from(value)
        .map(DeclValue::Int)
        .map_err(|_| catalog_invalid("Native weapon declaration integer exceeds i64"))
}

/// Compat declaration value for a `usize` field.
fn decl_usize(value: usize) -> Result<DeclValue, CatalogError> {
    i64::try_from(value)
        .map(DeclValue::Int)
        .map_err(|_| catalog_invalid("Native weapon declaration integer exceeds i64"))
}

/// Compat declaration value for a built-in entry.
fn q2eaks_entry_value(entry: &Q2eaksEntry) -> Result<DeclValue, CatalogError> {
    Ok(DeclValue::Map(
        [
            ("rva".to_owned(), decl_int(entry.rva)?),
            (
                "registration".to_owned(),
                entry
                    .registration
                    .as_ref()
                    .map(q2eaks_registration_value)
                    .transpose()?
                    .unwrap_or(DeclValue::Null),
            ),
        ]
        .into_iter()
        .collect(),
    ))
}

/// Compat declaration value for a built-in registration.
fn q2eaks_registration_value(registration: &Q2eaksRegistration) -> Result<DeclValue, CatalogError> {
    Ok(DeclValue::Map(
        [
            ("rva".to_owned(), decl_int(registration.rva)?),
            ("name".to_owned(), DeclValue::Str(registration.name.clone())),
            ("tag".to_owned(), decl_int(u64::from(registration.tag))?),
            (
                "layout".to_owned(),
                DeclValue::Map(
                    [
                        ("byteLength".to_owned(), decl_usize(registration.layout.byte_length)?),
                        ("name".to_owned(), decl_usize(registration.layout.name)?),
                        ("tag".to_owned(), decl_usize(registration.layout.tag)?),
                        ("callback".to_owned(), decl_usize(registration.layout.callback)?),
                    ]
                    .into_iter()
                    .collect(),
                ),
            ),
        ]
        .into_iter()
        .collect(),
    ))
}

/// Compat declaration value for a built-in call chain.
fn q2eaks_calls_value(calls: &[Q2eaksEntry]) -> Result<DeclValue, CatalogError> {
    Ok(DeclValue::Map(
        [
            ("signature".to_owned(), DeclValue::Str("entity-void".to_owned())),
            (
                "calls".to_owned(),
                DeclValue::List(calls.iter().map(q2eaks_entry_value).collect::<Result<Vec<_>, _>>()?),
            ),
        ]
        .into_iter()
        .collect(),
    ))
}

/// Compat declaration value for a built-in command.
fn q2eaks_command_value(command: &Q2eaksCommand) -> DeclValue {
    DeclValue::Map(
        [
            (
                "arguments".to_owned(),
                DeclValue::List(command.arguments.iter().cloned().map(DeclValue::Str).collect()),
            ),
            ("tail".to_owned(), DeclValue::Str(command.tail.clone())),
        ]
        .into_iter()
        .collect(),
    )
}

/// Compat declaration value for a built-in cvar.
fn q2eaks_cvar_value(cvar: &Q2eaksCvar) -> DeclValue {
    DeclValue::Map(
        [
            ("name".to_owned(), DeclValue::Str(cvar.name.clone())),
            ("value".to_owned(), DeclValue::Str(cvar.value.clone())),
        ]
        .into_iter()
        .collect(),
    )
}

/// Compat declaration value for the built-in Q2Eaks declaration.
fn q2eaks_declaration_value(declaration: &Q2eaksDeclaration) -> Result<DeclValue, CatalogError> {
    let entity = &declaration.entity;
    let client = &declaration.client;
    let equipped = &declaration.equipped_weapon;
    let fields = [
        ("version".to_owned(), decl_int(u64::from(declaration.version))?),
        ("kind".to_owned(), DeclValue::Str(declaration.kind.clone())),
        ("abi".to_owned(), DeclValue::Str(declaration.abi.clone())),
        (
            "artifactPath".to_owned(),
            DeclValue::Str(declaration.artifact_path.clone()),
        ),
        (
            "artifactDigest".to_owned(),
            DeclValue::Str(declaration.artifact_digest.clone()),
        ),
        ("id".to_owned(), DeclValue::Str(declaration.id.clone())),
        ("title".to_owned(), DeclValue::Str(declaration.title.clone())),
        ("role".to_owned(), DeclValue::Str(declaration.role.clone())),
        ("aspect".to_owned(), DeclValue::Str(declaration.aspect.clone())),
        (
            "entity".to_owned(),
            DeclValue::Map(
                [
                    ("byteLength".to_owned(), decl_usize(entity.byte_length)?),
                    ("origin".to_owned(), decl_usize(entity.origin)?),
                    ("angles".to_owned(), decl_usize(entity.angles)?),
                    ("velocity".to_owned(), decl_usize(entity.velocity)?),
                    ("client".to_owned(), decl_usize(entity.client)?),
                    ("owner".to_owned(), decl_usize(entity.owner)?),
                    ("viewHeight".to_owned(), decl_usize(entity.view_height)?),
                    ("generation".to_owned(), decl_usize(entity.generation)?),
                    ("nextThink".to_owned(), decl_usize(entity.next_think)?),
                    ("thinkCallback".to_owned(), decl_usize(entity.think_callback)?),
                    ("thinkRegistration".to_owned(), decl_usize(entity.think_registration)?),
                    ("touchCallback".to_owned(), decl_usize(entity.touch_callback)?),
                ]
                .into_iter()
                .collect(),
            ),
        ),
        (
            "client".to_owned(),
            DeclValue::Map(
                [
                    ("byteLength".to_owned(), decl_usize(client.byte_length)?),
                    ("weapon".to_owned(), decl_usize(client.weapon)?),
                    ("viewAngles".to_owned(), decl_usize(client.view_angles)?),
                    ("forward".to_owned(), decl_usize(client.forward)?),
                ]
                .into_iter()
                .collect(),
            ),
        ),
        (
            "equippedWeapon".to_owned(),
            DeclValue::Map(
                [
                    ("byteLength".to_owned(), decl_usize(equipped.byte_length)?),
                    ("callback".to_owned(), decl_usize(equipped.callback)?),
                    ("expected".to_owned(), q2eaks_entry_value(&equipped.expected)?),
                ]
                .into_iter()
                .collect(),
            ),
        ),
        (
            "time".to_owned(),
            DeclValue::Map(
                [
                    ("storage".to_owned(), DeclValue::Str(declaration.time_storage.clone())),
                    ("rva".to_owned(), decl_int(declaration.time_rva)?),
                ]
                .into_iter()
                .collect(),
            ),
        ),
        (
            "think".to_owned(),
            DeclValue::Map(
                [
                    (
                        "signature".to_owned(),
                        DeclValue::Str(declaration.think_signature.clone()),
                    ),
                    ("tag".to_owned(), decl_int(u64::from(declaration.think_tag))?),
                    (
                        "registration".to_owned(),
                        DeclValue::Map(
                            [
                                (
                                    "byteLength".to_owned(),
                                    decl_usize(declaration.think_registration.byte_length)?,
                                ),
                                ("name".to_owned(), decl_usize(declaration.think_registration.name)?),
                                ("tag".to_owned(), decl_usize(declaration.think_registration.tag)?),
                                (
                                    "callback".to_owned(),
                                    decl_usize(declaration.think_registration.callback)?,
                                ),
                            ]
                            .into_iter()
                            .collect(),
                        ),
                    ),
                ]
                .into_iter()
                .collect(),
            ),
        ),
        (
            "allocate".to_owned(),
            DeclValue::Map(
                [
                    (
                        "signature".to_owned(),
                        DeclValue::Str(declaration.allocate_signature.clone()),
                    ),
                    ("entry".to_owned(), q2eaks_entry_value(&declaration.allocate)?),
                ]
                .into_iter()
                .collect(),
            ),
        ),
        (
            "free".to_owned(),
            DeclValue::Map(
                [
                    (
                        "signature".to_owned(),
                        DeclValue::Str(declaration.free_signature.clone()),
                    ),
                    ("entry".to_owned(), q2eaks_entry_value(&declaration.free)?),
                ]
                .into_iter()
                .collect(),
            ),
        ),
        (
            "projectileTouch".to_owned(),
            q2eaks_entry_value(&declaration.projectile_touch)?,
        ),
        ("equip".to_owned(), q2eaks_calls_value(&declaration.equip)?),
        ("launch".to_owned(), q2eaks_calls_value(&declaration.launch)?),
        (
            "activateRva".to_owned(),
            declaration
                .activate_rva
                .map(decl_int)
                .transpose()?
                .unwrap_or(DeclValue::Null),
        ),
        ("fireRva".to_owned(), decl_int(declaration.fire_rva)?),
        (
            "initializationClasses".to_owned(),
            DeclValue::List(
                declaration
                    .initialization_classes
                    .iter()
                    .cloned()
                    .map(DeclValue::Str)
                    .collect(),
            ),
        ),
        (
            "equipment".to_owned(),
            DeclValue::List(declaration.equipment.iter().map(q2eaks_command_value).collect()),
        ),
        ("ammunition".to_owned(), q2eaks_command_value(&declaration.ammunition)),
        (
            "initialCvars".to_owned(),
            DeclValue::List(declaration.initial_cvars.iter().map(q2eaks_cvar_value).collect()),
        ),
        (
            "provisioningCvars".to_owned(),
            DeclValue::List(declaration.provisioning_cvars.iter().map(q2eaks_cvar_value).collect()),
        ),
    ];
    Ok(DeclValue::Map(fields.into_iter().collect()))
}

/// Contract projectile role for a validated declaration role name.
fn declaration_role(role: &str) -> Result<ProjectileRole, CatalogError> {
    match role {
        "rocket" => Ok(ProjectileRole::Rocket),
        "grenade" => Ok(ProjectileRole::Grenade),
        "nail" => Ok(ProjectileRole::Nail),
        "bolt" => Ok(ProjectileRole::Bolt),
        "plasma" => Ok(ProjectileRole::Plasma),
        "energy" => Ok(ProjectileRole::Energy),
        "grapple" => Ok(ProjectileRole::Grapple),
        _ => Err(catalog_invalid("Native weapon declaration carries an unknown role")),
    }
}

/// Contract offset for a validated layout offset.
fn layout_offset(value: usize) -> Result<u64, CatalogError> {
    u64::try_from(value).map_err(|_| catalog_invalid("Native weapon declaration offset exceeds u64"))
}

/// Contract registration layout for a validated compat layout.
fn contract_registration_layout(
    layout: &qa_compat::q2::rerelease::native_weapon_declaration::DeclRegistrationLayout,
) -> Result<NativeWeaponRegistrationLayout, CatalogError> {
    Ok(NativeWeaponRegistrationLayout {
        byte_length: layout_offset(layout.byte_length)?,
        name: layout_offset(layout.name)?,
        tag: layout_offset(layout.tag)?,
        callback: layout_offset(layout.callback)?,
    })
}

/// Contract entry for a validated compat entry.
fn contract_native_entry(
    entry: &qa_compat::q2::rerelease::native_weapon_declaration::DeclEntry,
) -> Result<NativeWeaponEntry, CatalogError> {
    Ok(NativeWeaponEntry {
        rva: entry.rva,
        registration: entry
            .registration
            .as_ref()
            .map(|registration| {
                Ok::<_, CatalogError>(NativeWeaponRegistration {
                    rva: registration.rva,
                    name: registration.name.clone(),
                    tag: u64::from(registration.tag),
                    layout: contract_registration_layout(&registration.layout)?,
                })
            })
            .transpose()?,
    })
}

/// Contract call list for a validated compat call chain.
fn contract_native_calls(
    calls: &qa_compat::q2::rerelease::native_weapon_declaration::DeclCalls,
) -> Result<NativeWeaponCalls, CatalogError> {
    Ok(NativeWeaponCalls {
        calls: calls
            .calls
            .iter()
            .map(contract_native_entry)
            .collect::<Result<Vec<_>, _>>()?,
    })
}

/// Contract declaration for a validated compat declaration. The reader
/// already checked the validated constants (`kind`, `abi`, `aspect`,
/// signatures, storage), so only the carried layout tables are mapped.
fn compat_declaration_to_contract(
    declaration: &CompatNativeDeclaration,
) -> Result<NativeWeaponBehaviorDeclaration, CatalogError> {
    Ok(NativeWeaponBehaviorDeclaration {
        version: declaration.version,
        id: declaration.id.clone(),
        title: declaration.title.clone(),
        role: declaration_role(&declaration.role)?,
        artifact_path: declaration.artifact_path.clone(),
        artifact_digest: ContentDigest(declaration.artifact_digest.clone()),
        entity: NativeWeaponEntity {
            byte_length: layout_offset(declaration.entity.byte_length)?,
            origin: layout_offset(declaration.entity.origin)?,
            angles: layout_offset(declaration.entity.angles)?,
            velocity: layout_offset(declaration.entity.velocity)?,
            client: layout_offset(declaration.entity.client)?,
            owner: layout_offset(declaration.entity.owner)?,
            view_height: layout_offset(declaration.entity.view_height)?,
            generation: layout_offset(declaration.entity.generation)?,
            next_think: layout_offset(declaration.entity.next_think)?,
            think_callback: layout_offset(declaration.entity.think_callback)?,
            think_registration: layout_offset(declaration.entity.think_registration)?,
            touch_callback: layout_offset(declaration.entity.touch_callback)?,
        },
        client: NativeWeaponClient {
            byte_length: layout_offset(declaration.client.byte_length)?,
            weapon: layout_offset(declaration.client.weapon)?,
            view_angles: layout_offset(declaration.client.view_angles)?,
            forward: layout_offset(declaration.client.forward)?,
        },
        equipped_weapon: NativeWeaponEquipped {
            byte_length: layout_offset(declaration.equipped_weapon.byte_length)?,
            callback: layout_offset(declaration.equipped_weapon.callback)?,
            expected: contract_native_entry(&declaration.equipped_weapon.expected)?,
        },
        time: NativeWeaponTime {
            rva: declaration.time_rva,
        },
        think: NativeWeaponThink {
            tag: u64::from(declaration.think_tag),
            registration: contract_registration_layout(&declaration.think_registration)?,
        },
        allocate: NativeWeaponAllocate {
            entry: contract_native_entry(&declaration.allocate)?,
        },
        free: NativeWeaponFree {
            entry: contract_native_entry(&declaration.free)?,
        },
        projectile_touch: contract_native_entry(&declaration.projectile_touch)?,
        equip: contract_native_calls(&declaration.equip)?,
        launch: contract_native_calls(&declaration.launch)?,
        activate_rva: declaration.activate_rva,
        fire_rva: declaration.fire_rva,
        initialization_classes: declaration.initialization_classes.clone(),
        equipment: declaration
            .equipment
            .iter()
            .map(|command| NativeWeaponCommand {
                arguments: command.arguments.clone(),
                tail: command.tail.clone(),
            })
            .collect(),
        ammunition: NativeWeaponCommand {
            arguments: declaration.ammunition.arguments.clone(),
            tail: declaration.ammunition.tail.clone(),
        },
        initial_cvars: declaration
            .initial_cvars
            .iter()
            .map(|cvar| NativeWeaponCvar {
                name: cvar.name.clone(),
                value: cvar.value.clone(),
            })
            .collect(),
        provisioning_cvars: declaration
            .provisioning_cvars
            .iter()
            .map(|cvar| NativeWeaponCvar {
                name: cvar.name.clone(),
                value: cvar.value.clone(),
            })
            .collect(),
    })
}

/// Compat image mirror for a parsed PE file (donor `parsePe` image shape
/// behind `validateNativeWeaponImage`).
fn pe_image_mirror(image: &qa_guest::pe::format::PeFile) -> PeImageMirror {
    PeImageMirror {
        abi: GuestNativeCallAbi::of(image.abi).kind().to_owned(),
        image_size: u64::from(image.image_size),
        sections: image
            .sections
            .iter()
            .map(|section| PeSectionMirror {
                rva: u64::from(section.rva),
                mapped_size: u64::from(section.mapped_size),
                permissions: [GuestAccess::Read, GuestAccess::Write, GuestAccess::Execute]
                    .into_iter()
                    .filter(|access| section.permissions.allows(*access))
                    .map(|access| access.label().to_owned())
                    .collect(),
            })
            .collect(),
    }
}

/// QuakeC mod callback declaration for a gameplay declaration value:
/// the shared JSON document converts to text and reads through the
/// canonical `readModCallbacks` home.
fn quakec_callback_declaration(reader: SaveReader) -> Result<ModCallbackDeclaration, ModsError> {
    let value = reader.value.cloned().unwrap_or(SaveJson::Null);
    let document = save_json_to_json(&value).map_err(ModsError::Invalid)?;
    read_mod_callbacks(stringify(&document).as_bytes()).map_err(|error| match error {
        qa_content::q1::mods_callbacks::ModCallbacksError::Mods(error) => error,
        qa_content::q1::mods_callbacks::ModCallbacksError::Value(error) => ModsError::Value(error),
        qa_content::q1::mods_callbacks::ModCallbacksError::Path(error) => ModsError::Path(error),
        qa_content::q1::mods_callbacks::ModCallbacksError::HeldWeapon(error) => ModsError::HeldWeapon(error),
        qa_content::q1::mods_callbacks::ModCallbacksError::ItemIcon(error) => ModsError::ItemIcon(error),
        qa_content::q1::mods_callbacks::ModCallbacksError::Qc(error) => ModsError::Invalid(error.to_string()),
    })
}

/// Convert a content value to output JSON.
fn save_json_to_json(value: &SaveJson) -> Result<Json, String> {
    match value {
        SaveJson::Null => Ok(Json::Null),
        SaveJson::Bool(value) => Ok(Json::Bool(*value)),
        SaveJson::Number(value) => Ok(Json::Number(*value)),
        SaveJson::BigInt(value) => {
            if *value >= -(1 << 53) && *value <= (1 << 53) {
                #[allow(clippy::cast_precision_loss)]
                Ok(Json::Number(*value as f64))
            } else {
                Err("Callback declaration number is outside the exact JSON integer range".to_owned())
            }
        }
        SaveJson::Bytes(_) => Err("Callback declaration stores raw bytes; expected JSON".to_owned()),
        SaveJson::String(value) => Ok(Json::String(value.clone())),
        SaveJson::Array(items) => Ok(Json::Array(
            items.iter().map(save_json_to_json).collect::<Result<Vec<_>, _>>()?,
        )),
        SaveJson::Object(members) => Ok(Json::Object(
            members
                .iter()
                .map(|(name, member)| save_json_to_json(member).map(|json| (name.clone(), json)))
                .collect::<Result<Vec<_>, _>>()?,
        )),
    }
}

/// Recipe resource reference for a contract resource reference.
fn recipe_resource(reference: &ResolvedResourceReference) -> Result<RecipeResourceReference, MountError> {
    Ok(RecipeResourceReference {
        id: reference.id.as_str().to_owned(),
        requested_path: reference.requested_path.clone(),
        provenance: match &reference.provenance {
            ContractProvenance::Archive {
                mount,
                member_path,
                member_index,
            } => RecipeProvenance::Archive {
                mount: Box::new(RecipeMount::Archive {
                    identity: RecipeMountIdentity {
                        id: mount.identity.id.as_str().to_owned(),
                        content: mount.identity.content.as_str().to_owned(),
                        generation: mount.identity.generation,
                    },
                    format: archive_format_name(&mount.format).to_owned(),
                    archive_path: mount.archive_path.clone(),
                    archive_digest: archive_digest_or_compute(mount)?.as_str().to_owned(),
                }),
                member_path: member_path.clone(),
                member_index: *member_index,
            },
            ContractProvenance::Loose { mount, member_path } => RecipeProvenance::Loose {
                mount: Box::new(RecipeMount::Loose {
                    identity: RecipeMountIdentity {
                        id: mount.identity.id.as_str().to_owned(),
                        content: mount.identity.content.as_str().to_owned(),
                        generation: mount.identity.generation,
                    },
                    root_path: mount.root_path.clone(),
                }),
                member_path: member_path.clone(),
            },
        },
        digest: reference.digest.as_str().to_owned(),
        byte_length: reference.byte_length,
        resolution: match &reference.resolution {
            ContractResolution::DefaultOrder { plan, rank } => RecipeResolution::DefaultOrder {
                plan: plan.as_str().to_owned(),
                rank: *rank,
            },
            ContractResolution::PrefixOrder { plan, prefix, rank } => RecipeResolution::PrefixOrder {
                plan: plan.as_str().to_owned(),
                rank: *rank,
                prefix: prefix.clone(),
            },
            ContractResolution::Link {
                plan,
                source_prefix,
                target_path,
            } => RecipeResolution::Link {
                plan: plan.as_str().to_owned(),
                source_prefix: source_prefix.clone(),
                target_path: target_path.clone(),
            },
        },
    })
}

/// Recipe archive format name for a contract archive format.
fn archive_format_name(format: &ArchiveFormat) -> &'static str {
    match format {
        ArchiveFormat::Pak => "pak",
        ArchiveFormat::Pk3 => "pk3",
        ArchiveFormat::Kpf => "kpf",
        ArchiveFormat::Zip => "zip",
    }
}

/// Convert a loaded QuakeC program into the behavior-resolution snapshot.
/// The donor (`weapon-behaviors.ts`) reads `QcProgram` directly; the
/// snapshot is the port's `qa-content` seam, so every row below mirrors the
/// donor field the resolution logic consumes.
fn weapon_snapshot(program: &QcProgram) -> QcWeaponProgramSnapshot {
    let view = QcProgramSnapshotView::new(program);
    QcWeaponProgramSnapshot {
        digest: ContentDigest(format!("{}:{}", program.digest.algorithm, program.digest.value)),
        capability_error: qc_weapon_behavior_capability_error(&view),
        functions: program
            .functions
            .iter()
            .map(|function| QcWeaponFunction {
                index: u32::try_from(function.index).unwrap_or(u32::MAX),
                name: function.name.clone(),
                first_statement: i64::from(function.first_statement),
                parameter_words: function.parameter_sizes.len(),
            })
            .collect(),
        statements: program
            .statements
            .iter()
            .map(|statement| QcWeaponStatement {
                opcode: match statement.opcode {
                    QcOpcode::Address => QcWeaponOpcode::Address,
                    QcOpcode::StorePFn => QcWeaponOpcode::StorePFn,
                    QcOpcode::StoreFn => QcWeaponOpcode::StoreFn,
                    _ => QcWeaponOpcode::Other,
                },
                a: i32::from(statement.a),
                b: i32::from(statement.b),
                c: i32::from(statement.c),
            })
            .collect(),
        think_field_offset: program
            .field_named("think")
            .and_then(|field| i32::try_from(field.offset).ok()),
        initial_global_words: program
            .initial_globals
            .as_chunks::<4>()
            .0
            .iter()
            .map(|word| i32::from_le_bytes(*word))
            .collect(),
        function_globals: program
            .globals
            .iter()
            .filter(|global| global.value_type == ProgramQcValueType::Function)
            .filter_map(|global| {
                i32::try_from(global.offset).ok().map(|offset| QcFunctionGlobal {
                    name: global.name.clone(),
                    offset,
                })
            })
            .collect(),
    }
}

/// [`QcProgramView`] over a loaded guest program for capability checks.
struct QcProgramSnapshotView<'a> {
    program: &'a QcProgram,
    digest: String,
}

impl<'a> QcProgramSnapshotView<'a> {
    fn new(program: &'a QcProgram) -> Self {
        Self {
            program,
            digest: format!("{}:{}", program.digest.algorithm, program.digest.value),
        }
    }
}

/// Map a program value type onto the provider view type.
fn snapshot_value_type(value: ProgramQcValueType) -> ModQcValueType {
    match value {
        ProgramQcValueType::Void => ModQcValueType::Void,
        ProgramQcValueType::String => ModQcValueType::String,
        ProgramQcValueType::Float => ModQcValueType::Float,
        ProgramQcValueType::Vector => ModQcValueType::Vector,
        ProgramQcValueType::Entity => ModQcValueType::Entity,
        ProgramQcValueType::Field => ModQcValueType::Field,
        ProgramQcValueType::Function => ModQcValueType::Function,
        ProgramQcValueType::Pointer => ModQcValueType::Pointer,
        ProgramQcValueType::Opaque => ModQcValueType::Opaque,
    }
}

/// Map a program function onto the provider view row.
fn snapshot_function_view(function: &QcFunction) -> QcFunctionView {
    QcFunctionView {
        index: i32::try_from(function.index).unwrap_or(i32::MAX),
        name: function.name.clone(),
        first_statement: function.first_statement,
        parameter_start: i32::try_from(function.parameter_start).unwrap_or(i32::MAX),
        parameter_sizes: function.parameter_sizes.iter().map(|size| i32::from(*size)).collect(),
        named_builtin: function.named_builtin,
    }
}

impl QcProgramView for QcProgramSnapshotView<'_> {
    fn digest(&self) -> &str {
        &self.digest
    }

    fn api_kind(&self) -> QcApiKind {
        match self.program.api {
            QuakeCApi::Netquake => QcApiKind::Q1Netquake,
            QuakeCApi::Quakeworld => QcApiKind::Q1Quakeworld,
        }
    }

    fn field_type(&self, name: &str) -> Option<ModQcValueType> {
        self.program
            .field_named(name)
            .map(|field| snapshot_value_type(field.value_type))
    }

    fn global_type(&self, name: &str) -> Option<ModQcValueType> {
        self.program
            .global_named(name)
            .map(|global| snapshot_value_type(global.value_type))
    }

    fn function_named(&self, name: &str) -> Option<QcFunctionView> {
        self.program.function_named(name).ok().map(snapshot_function_view)
    }

    fn function_at(&self, index: i32) -> Option<QcFunctionView> {
        usize::try_from(index)
            .ok()
            .and_then(|at| self.program.functions.get(at))
            .map(snapshot_function_view)
    }

    fn functions(&self) -> Vec<QcFunctionView> {
        self.program.functions.iter().map(snapshot_function_view).collect()
    }
}

/// List installed content discovered beneath the corpus root.
fn list_content(corpus_root: &str, stdout: &mut dyn Write) {
    let _ = writeln!(stdout, "corpus root: {corpus_root}");
    match discover_installed_content(&DiscoverContentOptions::new(corpus_root.into())) {
        Ok(catalog) => {
            for product in &catalog.products {
                let status = match &product.availability {
                    ProductAvailability::Installed => "installed".to_string(),
                    ProductAvailability::Missing { requirements } => {
                        format!("missing: {}", requirements.join(", "))
                    }
                    ProductAvailability::Unresolved { reason } => format!("unresolved: {reason}"),
                };
                let _ = writeln!(stdout, "{} [{}]", product.expectation.id, status);
                for archive in &product.archives {
                    let _ = writeln!(stdout, "  {}", archive.path);
                }
            }
        }
        Err(error) => {
            let _ = writeln!(stdout, "(catalog discovery failed: {error})");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::ApplicationOptions;
    use qa_guest::qc::program::load_qc_program;

    #[test]
    fn game_routing_covers_dedicated_and_frame_limits() {
        let base = ApplicationOptions::default();
        assert!(use_game_composition(&base));
        let mut windowed = base.clone();
        windowed.windowed = true;
        assert!(use_game_composition(&windowed));
        let mut windowed_frames = base.clone();
        windowed_frames.windowed = true;
        windowed_frames.frame_limit = Some(600);
        assert!(use_game_composition(&windowed_frames));
        // A bare frame cap still opens the game (and closes after N);
        // only dedicated serves headless.
        let mut capped = base.clone();
        capped.frame_limit = Some(20);
        assert!(use_game_composition(&capped));
        let mut dedicated = base.clone();
        dedicated.dedicated = true;
        assert!(!use_game_composition(&dedicated));
        let mut dedicated_frames = base.clone();
        dedicated_frames.dedicated = true;
        dedicated_frames.frame_limit = Some(20);
        assert!(!use_game_composition(&dedicated_frames));
    }

    /// Minimal version-6 `progs.dat`: two statements, one function global,
    /// one `think` field, a null function plus `fire_rocket`, and 28
    /// reserved global words.
    fn snapshot_program_bytes() -> Vec<u8> {
        fn push_i32(bytes: &mut Vec<u8>, value: i32) {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        fn push_u16(bytes: &mut Vec<u8>, value: u16) {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let mut bytes = Vec::new();
        push_i32(&mut bytes, 6);
        push_i32(&mut bytes, 5927);
        push_i32(&mut bytes, 60);
        push_i32(&mut bytes, 2);
        push_i32(&mut bytes, 76);
        push_i32(&mut bytes, 1);
        push_i32(&mut bytes, 84);
        push_i32(&mut bytes, 1);
        push_i32(&mut bytes, 92);
        push_i32(&mut bytes, 2);
        push_i32(&mut bytes, 164);
        push_i32(&mut bytes, 24);
        push_i32(&mut bytes, 188);
        push_i32(&mut bytes, 28);
        push_i32(&mut bytes, 16);
        debug_assert_eq!(bytes.len(), 60);
        push_u16(&mut bytes, 30);
        push_u16(&mut bytes, 1);
        push_u16(&mut bytes, 2);
        push_u16(&mut bytes, 3);
        push_u16(&mut bytes, 36);
        push_u16(&mut bytes, 4);
        push_u16(&mut bytes, 5);
        push_u16(&mut bytes, 6);
        debug_assert_eq!(bytes.len(), 76);
        push_u16(&mut bytes, 6);
        push_u16(&mut bytes, 9);
        push_i32(&mut bytes, 7);
        debug_assert_eq!(bytes.len(), 84);
        push_u16(&mut bytes, 6);
        push_u16(&mut bytes, 7);
        push_i32(&mut bytes, 1);
        debug_assert_eq!(bytes.len(), 92);
        for (first, name, file) in [(-1, 0, 0), (0, 7, 19)] {
            push_i32(&mut bytes, first);
            push_i32(&mut bytes, 0);
            push_i32(&mut bytes, 0);
            push_i32(&mut bytes, 0);
            push_i32(&mut bytes, name);
            push_i32(&mut bytes, file);
            push_i32(&mut bytes, 0);
            bytes.extend_from_slice(&[0; 8]);
        }
        debug_assert_eq!(bytes.len(), 164);
        bytes.extend_from_slice(b"\0think\0fire_rocket\0w.qc\0");
        debug_assert_eq!(bytes.len(), 188);
        for word in 0..28 {
            push_i32(&mut bytes, i32::from(word == 1));
        }
        bytes
    }

    fn run_text(argv: &[&str]) -> (i32, String, String) {
        let owned: Vec<String> = argv.iter().map(|arg| (*arg).to_string()).collect();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run(&owned, &mut stdout, &mut stderr, "0.1.0");
        (
            code,
            String::from_utf8(stdout).unwrap(),
            String::from_utf8(stderr).unwrap(),
        )
    }

    #[test]
    fn help_and_version_branches() {
        let (code, stdout, _) = run_text(&["--help"]);
        assert_eq!(code, 0);
        assert!(stdout.starts_with("Quake\n\nUsage: quake-anthology"));
        let (code, stdout, _) = run_text(&["--version"]);
        assert_eq!(code, 0);
        assert_eq!(stdout, "Quake Anthology 0.1.0\n");
    }

    #[test]
    fn dedicated_run_reports_stats() {
        // The success case needs a real Q1 corpus; skip loudly without one.
        let Ok(corpus) = std::env::var("QA_MUSE_Q1_CORPUS_PATH") else {
            eprintln!(
                "SKIP dedicated_run_reports_stats: set QA_MUSE_Q1_CORPUS_PATH to a corpus root with q1-classic-id1"
            );
            return;
        };
        let (code, stdout, _) = run_text(&[
            "--dedicated",
            "--content-root",
            corpus.as_str(),
            "--game",
            "q1-classic-id1",
            "--map",
            "start",
            "--frames",
            "4",
        ]);
        assert_eq!(code, 0);
        assert!(stdout.contains("4 host frames"), "{stdout}");
        assert!(stdout.contains("server ticks"), "{stdout}");
    }

    #[test]
    fn dedicated_without_content_refuses_at_cli() {
        let root = std::env::temp_dir().join(format!("qa-muse-no-content-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("empty root");
        let (code, _, stderr) = run_text(&[
            "--dedicated",
            "--content-root",
            root.to_str().expect("utf8 root"),
            "--game",
            "q1-classic-id1",
            "--map",
            "start",
            "--frames",
            "4",
        ]);
        assert_eq!(code, 1);
        assert!(stderr.contains("refuse to host a stub map"), "{stderr}");
    }

    #[test]
    fn windowed_unsupported_selections_fail_before_opening() {
        // `--renderer cpu` is supported (see the windowed CPU smoke test);
        // only worker rendering and dedicated conflicts still fail here.
        let (code, _, stderr) = run_text(&["--windowed", "--render-worker", "1", "--frames", "1"]);
        assert_eq!(code, 1);
        assert!(stderr.contains("worker"), "{stderr}");
        let (code, _, stderr) = run_text(&["--windowed", "--dedicated", "--frames", "1"]);
        assert_eq!(code, 1);
        assert!(stderr.contains("--windowed"), "{stderr}");
    }

    #[test]
    fn errors_exit_nonzero() {
        let (code, _, stderr) = run_text(&["--bogus", "x"]);
        assert_eq!(code, 1);
        assert!(stderr.contains("Unknown option"), "{stderr}");
        let (code, _, stderr) = run_text(&["--bogus"]);
        assert_eq!(code, 1);
        assert!(stderr.contains("Missing value"), "{stderr}");
        let (code, stdout, _) = run_text(&["weapon-behavior", "--help"]);
        assert_eq!(code, 0);
        assert!(stdout.contains("declare-qvm"), "{stdout}");
    }

    #[test]
    fn weapon_behavior_dispatch_reaches_tool() {
        let (code, _, stderr) = run_text(&["weapon-behavior", "inspect", "pkg"]);
        assert_eq!(code, 1);
        assert!(stderr.contains("Unknown requested content or mod: pkg"), "{stderr}");
        assert!(!stderr.contains("not ported yet"), "{stderr}");
    }

    #[test]
    fn tool_command_converts_every_action() {
        let content = crate::options::ToolContent {
            product: "q1".to_string(),
            corpus_root: "/corpus".to_string(),
            user_content_root: "/user".to_string(),
            artifact: Some("progs.dat".to_string()),
        };
        assert_eq!(
            tool_command(&ParsedWeaponBehaviorTool::Help),
            WeaponBehaviorToolCommand::Help
        );
        assert_eq!(
            tool_command(&ParsedWeaponBehaviorTool::Inspect {
                content: content.clone()
            }),
            WeaponBehaviorToolCommand::Inspect {
                product: "q1".to_string(),
                corpus_root: "/corpus".to_string(),
                user_content_root: "/user".to_string(),
                artifact: Some("progs.dat".to_string()),
            }
        );
        assert_eq!(
            tool_command(&ParsedWeaponBehaviorTool::DeclareQvm {
                content: content.clone(),
                profile: "prof.json".to_string(),
            }),
            WeaponBehaviorToolCommand::DeclareQvm {
                product: "q1".to_string(),
                corpus_root: "/corpus".to_string(),
                user_content_root: "/user".to_string(),
                artifact: Some("progs.dat".to_string()),
                profile: "prof.json".to_string(),
            }
        );
        assert_eq!(
            tool_command(&ParsedWeaponBehaviorTool::DeclareNative {
                content: content.clone(),
                profile: "prof.json".to_string(),
            }),
            WeaponBehaviorToolCommand::DeclareNative {
                product: "q1".to_string(),
                corpus_root: "/corpus".to_string(),
                user_content_root: "/user".to_string(),
                artifact: Some("progs.dat".to_string()),
                profile: "prof.json".to_string(),
            }
        );
        assert_eq!(
            tool_command(&ParsedWeaponBehaviorTool::Declare {
                content,
                id: "ns:rl".to_string(),
                title: "Rocket".to_string(),
                role: OptionsProjectileRole::Grapple,
                fire: "fire_grapple".to_string(),
                activate: Some("check".to_string()),
            }),
            WeaponBehaviorToolCommand::Declare {
                product: "q1".to_string(),
                corpus_root: "/corpus".to_string(),
                user_content_root: "/user".to_string(),
                artifact: Some("progs.dat".to_string()),
                id: "ns:rl".to_string(),
                title: "Rocket".to_string(),
                role: ToolProjectileRole::Grapple,
                fire: "fire_grapple".to_string(),
                activate: Some("check".to_string()),
            }
        );
    }

    #[test]
    fn tool_role_maps_every_role() {
        let pairs = [
            (OptionsProjectileRole::Rocket, ToolProjectileRole::Rocket),
            (OptionsProjectileRole::Grenade, ToolProjectileRole::Grenade),
            (OptionsProjectileRole::Nail, ToolProjectileRole::Nail),
            (OptionsProjectileRole::Bolt, ToolProjectileRole::Bolt),
            (OptionsProjectileRole::Plasma, ToolProjectileRole::Plasma),
            (OptionsProjectileRole::Energy, ToolProjectileRole::Energy),
            (OptionsProjectileRole::Grapple, ToolProjectileRole::Grapple),
        ];
        for (parsed, expected) in pairs {
            assert_eq!(tool_role(parsed), expected);
        }
    }

    #[test]
    fn weapon_snapshot_mirrors_program_rows() {
        let program = load_qc_program(&snapshot_program_bytes(), None, "test.dat").unwrap();
        let snapshot = weapon_snapshot(&program);
        assert!(snapshot.digest.as_str().starts_with("sha256:"));
        assert_eq!(snapshot.digest.as_str().len(), 7 + 64);
        assert!(snapshot
            .capability_error
            .is_some_and(|reason| reason.contains("QuakeC trajectory adapter requires")));
        assert_eq!(snapshot.functions.len(), 2);
        assert_eq!(snapshot.functions[1].index, 1);
        assert_eq!(snapshot.functions[1].name, "fire_rocket");
        assert_eq!(snapshot.functions[1].first_statement, 0);
        assert_eq!(snapshot.functions[1].parameter_words, 0);
        assert_eq!(snapshot.statements.len(), 2);
        assert_eq!(snapshot.statements[0].opcode, QcWeaponOpcode::Address);
        assert_eq!(
            (
                snapshot.statements[0].a,
                snapshot.statements[0].b,
                snapshot.statements[0].c
            ),
            (1, 2, 3)
        );
        assert_eq!(snapshot.statements[1].opcode, QcWeaponOpcode::StoreFn);
        assert_eq!(snapshot.think_field_offset, Some(7));
        assert_eq!(snapshot.initial_global_words.len(), 28);
        assert_eq!(snapshot.initial_global_words[0], 0);
        assert_eq!(snapshot.initial_global_words[1], 1);
        assert_eq!(snapshot.function_globals.len(), 1);
        assert_eq!(snapshot.function_globals[0].name, "fire_rocket");
        assert_eq!(snapshot.function_globals[0].offset, 9);
    }

    #[test]
    fn list_content_reports_catalog_products() {
        let root = std::env::temp_dir().join("quake-anthology-list-content");
        let _ = std::fs::create_dir_all(&root);
        let root = root.to_string_lossy().into_owned();
        let (code, stdout, _) = run_text(&["--list-content", "--content-root", root.as_str()]);
        assert_eq!(code, 0);
        assert!(stdout.contains(format!("corpus root: {root}").as_str()), "{stdout}");
        assert!(stdout.contains("q1-classic-id1"), "{stdout}");
        assert!(!stdout.contains("not ported yet"), "{stdout}");
    }

    fn qvm_test_module(digest: &str) -> ModuleIdentity {
        ModuleIdentity {
            id: ProviderId::new("qvm", "test"),
            artifact_path: "vm/qagame.qvm".to_owned(),
            digest: ContentDigest(digest.to_owned()),
            revision: "1".to_owned(),
        }
    }

    #[test]
    fn qvm_resolve_reaches_guest_parser() {
        let bytes = vec![0x7f; 128];
        let digest = format!("sha256:{}", qa_content::hash::sha256_hex(&bytes));
        let service = CliQvmService;
        let error = service
            .resolve_qvm_artifact(QvmAbiProfile::Modern, &bytes, &qvm_test_module(&digest))
            .expect_err("garbage bytes are not a QVM image");
        let message = error.to_string();
        assert!(message.contains("QVM"), "{message}");
        assert!(!message.contains("unported"), "{message}");
    }

    #[test]
    fn qvm_resolve_rejects_digest_mismatch() {
        let service = CliQvmService;
        let error = service
            .resolve_qvm_artifact(QvmAbiProfile::Modern, &[1, 2, 3, 4], &qvm_test_module("sha256:00"))
            .expect_err("mismatched digest is rejected");
        assert!(!error.to_string().contains("unported"), "{error}");
    }

    fn qvm_test_profile() -> CliQvmProfile {
        use qa_guest::qvm::mod_provider::ModuleId;
        use qa_guest::qvm::weapon_behavior_profile::{FireAbi, WeaponLayoutFields};
        let module = ModuleId {
            id: "qvm:test".to_owned(),
            artifact_path: "vm/qagame.qvm".to_owned(),
            digest: "sha256:00".to_owned(),
            revision: "1".to_owned(),
        };
        let contract_module = qvm_test_module("sha256:00");
        CliQvmProfile {
            profile: GuestQvmWeaponProfile {
                definition: GuestWeaponBehaviorDefinition {
                    id: "qvm:rl".to_owned(),
                    title: "Rocket".to_owned(),
                    role: GuestWeaponBehaviorRole::Rocket,
                    aspect: "trajectory".to_owned(),
                    module: module.clone(),
                    fire: GuestWeaponBehaviorCallback::Qvm {
                        module,
                        instruction_index: 3,
                    },
                    activate: None,
                },
                layout: qa_guest::qvm::weapon_behavior_profile::QvmWeaponBehaviorLayout {
                    entity_stride: 512,
                    level_time: 64,
                    allocate: 5,
                    free: 9,
                    fields: WeaponLayoutFields {
                        inuse: 0,
                        nextthink: 4,
                        think: 8,
                        health: 12,
                    },
                    fire_abi: FireAbi::EntityPointerStartDirection,
                },
            },
            definition: WeaponBehaviorDefinition {
                id: "qvm:rl".to_owned(),
                title: "Rocket".to_owned(),
                module: contract_module.clone(),
                role: ProjectileRole::Rocket,
                activate: None,
                fire: WeaponBehaviorCallback::Qvm {
                    module: contract_module,
                    instruction_index: 3,
                },
            },
        }
    }

    #[test]
    fn qvm_profile_id_and_surface_read_definition() {
        let service = CliQvmService;
        let profile = qvm_test_profile();
        assert_eq!(service.qvm_weapon_profile_id(&profile), "qvm:rl");
        assert_eq!(profile.behavior_definition().id, "qvm:rl");
        assert_eq!(profile.behavior_entity_stride(), 512);
        assert_eq!(profile.behavior_level_time(), 64);
        assert_eq!(profile.behavior_allocate(), 5);
        assert_eq!(profile.behavior_free(), 9);
        let fields = profile.behavior_fields();
        assert_eq!(
            (fields.inuse, fields.nextthink, fields.think, fields.health),
            (0, 4, 8, 12)
        );
    }

    #[test]
    fn native_read_rejects_malformed_document() {
        let service = CliNativeService;
        let error = service
            .read_native_weapon_declaration(&SaveJson::Object(Vec::new()))
            .expect_err("an empty document has no declaration keys");
        let message = error.to_string();
        assert!(message.contains("native-weapon-profile"), "{message}");
        assert!(!message.contains("unported"), "{message}");
    }

    fn native_test_module(digest: &str) -> ModuleIdentity {
        ModuleIdentity {
            id: ProviderId::new("q2", "rerelease"),
            artifact_path: "game_x64.dll".to_owned(),
            digest: ContentDigest(digest.to_owned()),
            revision: digest.to_owned(),
        }
    }

    #[test]
    fn native_builtin_matches_exact_artifact() {
        let service = CliNativeService;
        let declaration = service
            .builtin_rerelease_weapon_declaration(&native_test_module(Q2EAKS_WEAPON_DIGEST))
            .expect("builtin reads")
            .expect("exact artifact has a builtin declaration");
        assert_eq!(declaration.id, "native:rocket-trajectory");
        assert_eq!(declaration.role, ProjectileRole::Rocket);
        assert_eq!(declaration.fire_rva, 0xef900);
        assert_eq!(declaration.activate_rva, Some(0xed4d0));
        let foreign = service
            .builtin_rerelease_weapon_declaration(&native_test_module("sha256:00"))
            .expect("foreign digest reads");
        assert!(foreign.is_none());
    }

    #[test]
    fn native_definition_requires_executable_image() {
        let service = CliNativeService;
        let module = native_test_module(Q2EAKS_WEAPON_DIGEST);
        let declaration = service
            .builtin_rerelease_weapon_declaration(&module)
            .expect("builtin reads")
            .expect("builtin declaration");
        let error = service
            .native_weapon_definition(&declaration, &module, b"not a pe image")
            .expect_err("garbage bytes are not a PE image");
        let message = error.to_string();
        assert!(message.contains("PE"), "{message}");
        assert!(!message.contains("unported"), "{message}");
    }

    fn temp_roots(name: &str) -> (String, String) {
        let root = std::env::temp_dir().join(name);
        let _ = std::fs::create_dir_all(&root);
        let root = root.to_string_lossy().into_owned();
        (root.clone(), root)
    }

    #[test]
    fn content_mounts_reports_real_catalog_state() {
        let (corpus, user) = temp_roots("quake-anthology-cli-mounts");
        let mut host = CliWeaponBehaviorHost::with_roots(corpus, user);
        let error = host
            .content_mounts(&ContentId("no-such-content".to_owned()))
            .expect_err("unknown content is rejected");
        let message = error.to_string();
        assert!(!message.contains("unported"), "{message}");
    }

    fn test_provider(content: &str) -> ProviderReference {
        ProviderReference {
            provider: ProviderId::new("q1", "test"),
            content: ContentId(content.to_owned()),
        }
    }

    fn test_recipe() -> ExecutableRecipe {
        use qa_content::contract::{
            CampaignSelection, CharacterSelection, DopplerSelection, EnemySelection, EnvironmentSelection,
            EquipmentSelection, FrameOrdering, GrappleSelection, HandGrenadeSelection, LooseMount, MountId,
            MountIdentity, MountPlanId, PresentationSelection, RecipeId, ResolvedMap, ResourceId, ResourceProvenance,
            ResourceResolution,
        };
        let geometry = ResolvedResourceReference {
            id: ResourceId("resource:maps/e1m1.bsp".to_owned()),
            requested_path: "maps/e1m1.bsp".to_owned(),
            provenance: ResourceProvenance::Loose {
                mount: LooseMount {
                    identity: MountIdentity {
                        id: MountId("mount:test:loose".to_owned()),
                        content: ContentId("q1".to_owned()),
                        generation: 1,
                    },
                    root_path: "/corpus".to_owned(),
                },
                member_path: "maps/e1m1.bsp".to_owned(),
            },
            digest: ContentDigest("sha256:00".to_owned()),
            byte_length: 0,
            resolution: ResourceResolution::DefaultOrder {
                plan: MountPlanId("mount-plan:test:1".to_owned()),
                rank: 0,
            },
        };
        ExecutableRecipe {
            weapon_behaviors: Vec::new(),
            mods: Vec::new(),
            schema_version: 3,
            id: RecipeId("recipe:test:1".to_owned()),
            preset: RecipeId("recipe:test:1".to_owned()),
            map: ResolvedMap {
                geometry_content: ContentId("q1".to_owned()),
                geometry: geometry.clone(),
                entities: test_provider("q1"),
            },
            campaign: CampaignSelection::None,
            movement: test_provider("q1"),
            character: CharacterSelection {
                definition: test_provider("q1"),
                appearance: test_provider("q1"),
            },
            weapons: Vec::new(),
            equipment: EquipmentSelection {
                grapple: GrappleSelection::Disabled,
                hand_grenades: HandGrenadeSelection::Disabled,
            },
            enemies: EnemySelection::MapDefined,
            presentation: PresentationSelection {
                doppler: DopplerSelection::Source,
                environment: EnvironmentSelection::AudioContent,
                assets: ContentId("q1".to_owned()),
                hud: test_provider("q1"),
                effects: test_provider("q1"),
                audio: test_provider("q1"),
            },
            engine_behavior: test_provider("q1"),
            combat: test_provider("q1"),
            inventory: test_provider("q1"),
            r#match: test_provider("q1"),
            transition: test_provider("q1"),
            execution: Vec::new(),
            mounts: ResolvedMountPlan {
                id: MountPlanId("mount-plan:test:1".to_owned()),
                mounts: Vec::new(),
                default_order: Vec::new(),
                prefix_orders: Vec::new(),
            },
            resources: vec![geometry],
            timing: Vec::new(),
            ordering: FrameOrdering::Mixed { providers: Vec::new() },
        }
    }

    #[test]
    fn apply_requests_empty_keeps_recipe() {
        let (corpus, user) = temp_roots("quake-anthology-cli-apply");
        let catalog = discover_installed_content(&DiscoverContentOptions {
            corpus_root: PathBuf::from(&corpus),
            user_content_root: Some(PathBuf::from(&user)),
            products: None,
            generation: 0,
            discover_mods: true,
            remote_content: None,
        })
        .expect("catalog discovers");
        let recipe = test_recipe();
        let mut host = CliWeaponBehaviorHost::default();
        let applied = host
            .apply_requests(&catalog, recipe.clone(), &[])
            .expect("empty applies");
        assert_eq!(applied.mods, recipe.mods);
        assert_eq!(applied.weapon_behaviors, recipe.weapon_behaviors);
    }

    fn empty_mounts() -> MountedContent {
        use qa_content::contract::MountPlanId;
        open_mount_plan(
            &ResolvedMountPlan {
                id: MountPlanId("mount-plan:test:empty".to_owned()),
                mounts: Vec::new(),
                default_order: Vec::new(),
                prefix_orders: Vec::new(),
            },
            OpenMountOptions::default(),
        )
        .expect("empty plan opens")
    }

    #[test]
    fn prepare_quakec_resources_scans_program() {
        use qa_guest::qc::program::load_qc_program;
        let program = load_qc_program(&snapshot_program_bytes(), None, "test.dat").unwrap();
        let mounts = empty_mounts();
        let mut host = CliWeaponBehaviorHost::default();
        let resources = host
            .prepare_quakec_resources(&program, &mounts)
            .expect("program without asset names prepares");
        assert!(resources.is_empty());
    }

    #[test]
    fn prepare_rerelease_guest_checks_artifact_digest() {
        use qa_content::contract::{LooseMount, MountId, MountIdentity, MountPlanId, ResourceId};
        let artifact = ResolvedResourceReference {
            id: ResourceId("resource:game_x64.dll".to_owned()),
            requested_path: "game_x64.dll".to_owned(),
            provenance: ContractProvenance::Loose {
                mount: LooseMount {
                    identity: MountIdentity {
                        id: MountId("mount:test:loose".to_owned()),
                        content: ContentId("q2-rerelease".to_owned()),
                        generation: 1,
                    },
                    root_path: "/corpus".to_owned(),
                },
                member_path: "game_x64.dll".to_owned(),
            },
            digest: ContentDigest("sha256:00".to_owned()),
            byte_length: 4,
            resolution: ContractResolution::DefaultOrder {
                plan: MountPlanId("mount-plan:test:1".to_owned()),
                rank: 0,
            },
        };
        let mounts = empty_mounts();
        let mut host = CliWeaponBehaviorHost::default();
        let error = host
            .prepare_rerelease_guest(&test_provider("q2-rerelease"), &artifact, b"fake", &mounts)
            .expect_err("digest mismatch is rejected");
        let message = error.to_string();
        assert!(message.contains("digest"), "{message}");
        assert!(!message.contains("unported"), "{message}");
    }
}
