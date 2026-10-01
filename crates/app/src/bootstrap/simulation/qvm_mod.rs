//! QVM gameplay mod preparation.
//!
//! Provenance: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/qvm-mod.ts`
//! (donor `prepareQvmMod`, `prepareMountedQvmMod`).
//!
//! The donor takes the contracts declaration; the worktree splits it into the
//! persisted content shape (`qa_content::contract::QvmModCallbackDeclaration`)
//! and the validated runtime shape this file takes
//! (`qa_guest::qvm::mod_provider::QvmModCallbackDeclaration`). Contract to
//! runtime conversion belongs to the mod-selection lane, which owns the
//! objective/pickup/combat sub-declarations; this file prepares an already
//! selected runtime declaration for execution.
//!
//! The donor `QvmModProvider` concrete class has no ported home; the guest
//! lane ported the generic `QvmModProvider<H: ModProviderHost>` over
//! session-owned hosts. This file owns artifact resolution, digest checks,
//! presentation assembly, checkpoint validation, and the instance lifecycle,
//! and injects the VM module state plus the cgame presentation source through
//! the [`QvmModModuleState`] and [`ModPresentationSource`] seams.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use qa_content::contract::borrow_mod_file_mounts;
use qa_content::contract::ContentDigest;
use qa_content::contract::ModDescription;
use qa_content::contract::ModIdentity;
use qa_content::contract::ModuleIdentity as ContractModule;
use qa_content::contract::ProviderCheckpoint;
use qa_content::mounts::MountedContent;
use qa_core::identity::ActorId;
use qa_core::identity::SavedActorId;
use qa_core::math::Vec3;
use qa_core::time::FrameContext;
use qa_core::time::SourceTime;
use qa_guest::core::contracts::ContentDigest as GuestDigest;
use qa_guest::core::contracts::ModuleIdentity as GuestModule;
use qa_guest::qvm::artifacts::resolve_qvm_artifact;
use qa_guest::qvm::artifacts::ResolvedQvmArtifact;
use qa_guest::qvm::mod_presentation::validate_qvm_mod_presentation;
use qa_guest::qvm::mod_presentation::HudMode;
use qa_guest::qvm::mod_presentation::QvmModPresentationDeclaration;
use qa_guest::qvm::mod_provider::validate_qvm_mod;
use qa_guest::qvm::mod_provider::validate_qvm_mod_checkpoint;
use qa_guest::qvm::mod_provider::CallbackResult;
use qa_guest::qvm::mod_provider::CallbackStage;
use qa_guest::qvm::mod_provider::ModCallbackBinding;
use qa_guest::qvm::mod_provider::ModCallbackInput as QvmInput;
use qa_guest::qvm::mod_provider::ModCallbackOperation as QvmOperation;
use qa_guest::qvm::mod_provider::ModPresentation;
use qa_guest::qvm::mod_provider::ModProviderHost;
use qa_guest::qvm::mod_provider::ModRuntimeValue as QvmValue;
use qa_guest::qvm::mod_provider::ModuleId;
use qa_guest::qvm::mod_provider::ProfileValue;
use qa_guest::qvm::mod_provider::QvmAbi;
use qa_guest::qvm::mod_provider::QvmArtifact as ProviderArtifact;
use qa_guest::qvm::mod_provider::QvmModCallbackDeclaration;
use qa_guest::qvm::mod_provider::QvmModCheckpointView;
use qa_guest::qvm::mod_provider::QvmModProvider;
use qa_guest::qvm::mod_provider::QvmRole as ProviderRole;
use qa_guest::qvm::syscalls::QvmAbiProfile as SysAbiProfile;
use qa_guest::qvm::syscalls::QvmRole as SysRole;
use qa_platform::files::writable::UserFileStore;

use super::mod_callbacks::register_mod_callbacks;
use super::mod_callbacks::DamageField;
use super::mod_callbacks::ModCallback as SimCallback;
use super::mod_callbacks::ModCallbackInput as SimInput;
use super::mod_callbacks::ModCallbackOperation as SimOperation;
use super::mod_callbacks::ModCallbackStage as SimStage;
use super::mod_callbacks::ModRegistrations;
use super::mod_callbacks::ModRuntimeValue as SimValue;
use super::qvm_weapon_behavior::convert_image;
use crate::bootstrap::simulation::types::ModQvmPresentation as HubPresentation;

/// QVM mod preparation failure.
#[derive(Debug, thiserror::Error)]
pub enum QvmModError {
    /// Guest failure.
    #[error(transparent)]
    Guest(#[from] qa_guest::error::GuestError),
    /// Contract failure.
    #[error(transparent)]
    Contract(#[from] qa_content::contract::ContractError),
    /// Mount failure.
    #[error(transparent)]
    Mount(#[from] qa_content::mounts::MountError),
    /// Resolved program differs from its artifact.
    #[error("Selected QVM mod differs from its resolved artifact")]
    SelectedDiffers,
    /// Resolved presentation differs from its artifact.
    #[error("Selected QVM mod presentation differs from its resolved artifact")]
    PresentationDiffers,
    /// Gameplay artifact is a native replacement.
    #[error("Gameplay mod callbacks require their authored QVM executable")]
    NativeGameplay,
    /// Presentation artifact is a native replacement.
    #[error("Mod presentation requires its authored QVM executable")]
    NativePresentation,
    /// Presentation declared without cgame bytes.
    #[error("Declared QVM mod presentation requires its authored cgame bytes")]
    MissingPresentationBytes,
    /// Cgame bytes without a presentation declaration.
    #[error("QVM mod presentation bytes require a declaration")]
    PresentationBytesWithoutDeclaration,
    /// Presentation gameplay ABI differs.
    #[error("QVM mod presentation differs from its gameplay ABI")]
    AbiMismatch,
    /// Services are unavailable.
    #[error("QVM gameplay mods require destination world services")]
    MissingServices,
    /// Checkpoint has no QVM guest.
    #[error("Missing QVM gameplay mod checkpoint")]
    MissingCheckpoint,
    /// Presentation source seam is missing.
    #[error("QVM mod presentation source requires its session host")]
    MissingPresentationSource,
    /// Digest text is not `algorithm:value`.
    #[error("QVM mod digest is not an algorithm:value pair")]
    BadDigest,
}

/// Options for [`prepare_qvm_mod`], mirroring donor `PrepareQvmModOptions`.
pub struct PrepareQvmModOptions {
    /// Mod description.
    pub description: ModDescription,
    /// Runtime callback declaration.
    pub declaration: QvmModCallbackDeclaration,
    /// Declaration digest.
    pub declaration_digest: ContentDigest,
    /// Qagame program bytes.
    pub program: Vec<u8>,
    /// Cgame presentation bytes, if declared.
    pub presentation_program: Option<Vec<u8>>,
    /// Installed content mounts.
    pub mounts: Option<MountedContent>,
}

/// Options for [`prepare_mounted_qvm_mod`].
pub struct PrepareMountedQvmModOptions {
    /// Mod description.
    pub description: ModDescription,
    /// Runtime callback declaration.
    pub declaration: QvmModCallbackDeclaration,
    /// Declaration digest.
    pub declaration_digest: ContentDigest,
    /// Installed content mounts.
    pub mounts: MountedContent,
}

/// Prepared QVM presentation, mirroring the donor `presentation` member.
#[derive(Debug)]
pub struct QvmModPresentation {
    /// Resolved cgame artifact.
    pub artifact: ResolvedQvmArtifact,
    /// Owning module.
    pub source: GuestModule,
    /// Presentation declaration.
    pub declaration: QvmModPresentationDeclaration,
}

/// Prepared HUD mode, mirroring donor `clientPresentation.hud`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmModHud {
    /// Overlay HUD.
    Overlay,
    /// Replacement HUD.
    Replace,
}

/// Prepared client presentation admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmModClientPresentation {
    /// HUD mode.
    pub hud: QvmModHud,
    /// View ownership (donor: always false for QVM mods).
    pub view: bool,
}

/// Prepared QVM gameplay mod.
pub struct PreparedQvmMod {
    /// Mod description.
    pub description: ModDescription,
    /// Mod identity.
    pub identity: ModIdentity,
    /// Module identity.
    pub module: GuestModule,
    /// Resolved qagame artifact.
    pub artifact: ResolvedQvmArtifact,
    /// Provider-layout artifact for validation.
    provider_artifact: ProviderArtifact,
    /// Runtime declaration.
    declaration: QvmModCallbackDeclaration,
    /// Content mounts.
    mounts: Option<MountedContent>,
    /// Prepared presentation.
    pub presentation: Option<QvmModPresentation>,
    /// Validated cgame bytes for hub re-resolution.
    presentation_program: Option<Vec<u8>>,
    /// Client presentation admission.
    pub client_presentation: Option<QvmModClientPresentation>,
}

impl std::fmt::Debug for PreparedQvmMod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedQvmMod")
            .field("description", &self.description)
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}

fn guest_digest(text: &str) -> Result<GuestDigest, QvmModError> {
    let (algorithm, value) = text.split_once(':').ok_or(QvmModError::BadDigest)?;
    Ok(GuestDigest::new(algorithm, value))
}

fn provider_role(role: SysRole) -> ProviderRole {
    match role {
        SysRole::Qagame => ProviderRole::Qagame,
        SysRole::Cgame => ProviderRole::Cgame,
        SysRole::Ui => ProviderRole::Ui,
    }
}

fn provider_abi(abi: SysAbiProfile) -> QvmAbi {
    match abi {
        SysAbiProfile::Modern => QvmAbi::Modern,
        SysAbiProfile::Legacy116n => QvmAbi::Legacy,
    }
}

fn syscall_abi(abi: QvmAbi) -> SysAbiProfile {
    match abi {
        QvmAbi::Modern => SysAbiProfile::Modern,
        QvmAbi::Legacy => SysAbiProfile::Legacy116n,
    }
}

fn contract_module(module: &GuestModule) -> ContractModule {
    ContractModule {
        id: module.id.clone(),
        artifact_path: module.artifact_path.clone(),
        digest: ContentDigest(format!("{}:{}", module.digest.algorithm, module.digest.value)),
        revision: module.revision.clone(),
    }
}

fn module_id(module: &GuestModule) -> ModuleId {
    ModuleId {
        id: format!("{}:{}", module.id.namespace, module.id.name),
        artifact_path: module.artifact_path.clone(),
        digest: format!("{}:{}", module.digest.algorithm, module.digest.value),
        revision: module.revision.clone(),
    }
}

/// Resolve bytecode and convert it to the provider layout the guest
/// validators consume.
fn resolve_bytecode(
    module: &GuestModule,
    role: SysRole,
    bytes: &[u8],
    abi: SysAbiProfile,
    native: QvmModError,
) -> Result<(ResolvedQvmArtifact, ProviderArtifact), QvmModError> {
    let resolved = resolve_qvm_artifact(module, role, bytes, Vec::new(), abi)?;
    let ResolvedQvmArtifact::Bytecode { image, .. } = &resolved else {
        return Err(native);
    };
    let artifact = ProviderArtifact {
        module: module_id(module),
        role: provider_role(role),
        abi_profile: Some(provider_abi(abi)),
        image: convert_image(image).1,
    };
    Ok((resolved, artifact))
}

/// Read a mounted program and reject digest drift.
fn open_program(mounts: &MountedContent, path: &str, digest: &str, drift: QvmModError) -> Result<Vec<u8>, QvmModError> {
    let found = mounts.open(path, |_| true)?;
    let Some(found) = found else {
        return Err(drift);
    };
    if found.reference.digest.as_str() != digest {
        return Err(drift);
    }
    Ok(found.bytes)
}

/// Prepare a QVM mod from mounted programs.
pub fn prepare_mounted_qvm_mod(options: PrepareMountedQvmModOptions) -> Result<PreparedQvmMod, QvmModError> {
    let program = open_program(
        &options.mounts,
        &options.declaration.program_path,
        &options.declaration.program_digest,
        QvmModError::SelectedDiffers,
    )?;
    let presentation_program = options
        .declaration
        .presentation
        .as_ref()
        .map(|presentation| {
            let cgame = presentation.cgame();
            open_program(
                &options.mounts,
                &cgame.path,
                &cgame.digest,
                QvmModError::PresentationDiffers,
            )
        })
        .transpose()?;
    prepare_qvm_mod(PrepareQvmModOptions {
        description: options.description,
        declaration: options.declaration,
        declaration_digest: options.declaration_digest,
        program,
        presentation_program,
        mounts: Some(options.mounts),
    })
}

/// Prepare a QVM mod from explicit program bytes.
pub fn prepare_qvm_mod(options: PrepareQvmModOptions) -> Result<PreparedQvmMod, QvmModError> {
    use qa_content::contract::mod_instance_provider;

    let provider = mod_instance_provider(&options.description.selection)?;
    let module = GuestModule::new(
        provider,
        &options.declaration.program_path,
        guest_digest(&options.declaration.program_digest)?,
        options.declaration_digest.as_str(),
    );
    let abi = syscall_abi(options.declaration.abi_profile);
    let (artifact, provider_artifact) = resolve_bytecode(
        &module,
        SysRole::Qagame,
        &options.program,
        abi,
        QvmModError::NativeGameplay,
    )?;
    validate_qvm_mod(&provider_artifact, &options.declaration)?;
    let mut presentation = None;
    if let Some(declared) = &options.declaration.presentation {
        let bytes = options
            .presentation_program
            .as_ref()
            .ok_or(QvmModError::MissingPresentationBytes)?;
        let gameplay = match declared {
            QvmModPresentationDeclaration::PlayerEvents(presentation) => &presentation.gameplay,
            QvmModPresentationDeclaration::Scene(presentation) => &presentation.gameplay,
        };
        if gameplay.abi != options.declaration.abi_profile {
            return Err(QvmModError::AbiMismatch);
        }
        let cgame = declared.cgame();
        let cgame_module = GuestModule::new(
            module.id.clone(),
            &cgame.path,
            guest_digest(&cgame.digest)?,
            options.declaration_digest.as_str(),
        );
        let cgame_abi = syscall_abi(cgame.abi);
        let (cgame_artifact, cgame_provider) = resolve_bytecode(
            &cgame_module,
            SysRole::Cgame,
            bytes,
            cgame_abi,
            QvmModError::NativePresentation,
        )?;
        validate_qvm_mod_presentation(&cgame_provider, &module_id(&module), declared)?;
        presentation = Some(QvmModPresentation {
            artifact: cgame_artifact,
            source: module.clone(),
            declaration: declared.clone(),
        });
    } else if options.presentation_program.is_some() {
        return Err(QvmModError::PresentationBytesWithoutDeclaration);
    }
    let client_presentation = options
        .declaration
        .presentation
        .as_ref()
        .and_then(|declared| presentation_hud(declared).map(|hud| QvmModClientPresentation { hud, view: false }));
    let identity = ModIdentity {
        selection: options.description.selection.clone(),
        source: options.description.source.clone(),
        declaration_digest: options.declaration_digest,
        modules: vec![contract_module(&module)],
        providers: Vec::new(),
    };
    Ok(PreparedQvmMod {
        description: options.description,
        identity,
        module,
        artifact,
        provider_artifact,
        declaration: options.declaration,
        mounts: options.mounts,
        presentation,
        presentation_program: options.presentation_program,
        client_presentation,
    })
}

fn presentation_hud(declared: &QvmModPresentationDeclaration) -> Option<QvmModHud> {
    let hud = match declared {
        QvmModPresentationDeclaration::PlayerEvents(presentation) => presentation.hud.as_ref(),
        QvmModPresentationDeclaration::Scene(presentation) => presentation.hud.as_ref(),
    };
    hud.map(|hud| match hud.mode {
        HudMode::Overlay => QvmModHud::Overlay,
        HudMode::ReplaceStatus => QvmModHud::Replace,
    })
}

impl PreparedQvmMod {
    /// Mod description.
    #[must_use]
    pub fn description(&self) -> &ModDescription {
        &self.description
    }

    /// Mod identity.
    #[must_use]
    pub fn identity(&self) -> &ModIdentity {
        &self.identity
    }

    /// Runtime declaration.
    #[must_use]
    pub fn declaration(&self) -> &QvmModCallbackDeclaration {
        &self.declaration
    }

    /// Hub presentation data, mirroring donor `presentation` for the
    /// simulation hub (`super::types::ModQvmPresentation`). The hub
    /// declaration is a unit placeholder; the real declaration stays on
    /// `self.presentation` until the lanes unify.
    #[must_use]
    pub fn hub_presentation(&self) -> Option<HubPresentation> {
        // Resolved artifacts are not cloneable; re-resolve the validated
        // cgame bytes for the hub handle.
        let presentation = self.presentation.as_ref()?;
        let bytes = self.presentation_program.as_ref()?;
        let cgame = presentation.declaration.cgame();
        let module = GuestModule::new(
            presentation.source.id.clone(),
            &cgame.path,
            GuestDigest::new("sha256", cgame.digest.trim_start_matches("sha256:")),
            &presentation.source.revision,
        );
        let resolved = resolve_qvm_artifact(&module, SysRole::Cgame, bytes, Vec::new(), syscall_abi(cgame.abi)).ok()?;
        Some(HubPresentation {
            artifact: resolved,
            source: contract_module(&presentation.source),
            declaration: super::types::QvmModPresentationDeclaration,
        })
    }

    /// Validate a decoded mod checkpoint against this preparation.
    pub fn validate_state(&self, state: &ModCheckpointState) -> Result<(), QvmModError> {
        let [guest] = state.guests.as_slice() else {
            return Err(QvmModError::MissingCheckpoint);
        };
        if !state.providers.is_empty() {
            return Err(QvmModError::MissingCheckpoint);
        }
        let view = QvmModCheckpointView {
            module: &guest.module.module,
            data: &guest.module.data,
            api_kind: &guest.module.api_kind,
            api_version: guest.module.api_version,
            abi: guest.module.abi,
            instruction_index: guest.module.instruction_index,
            operand_stack_len: guest.module.operand_stack.len(),
            program_stack: guest.module.program_stack,
            host_module: &guest.host_module,
            host_format: &guest.host_format,
            host: &guest.host,
            random_len: guest.module.random_len,
            callbacks_len: guest.module.callbacks_len,
        };
        validate_qvm_mod_checkpoint(&self.provider_artifact, &self.declaration, &view)?;
        Ok(())
    }
}

/// Saved VM module image: the machine-owned half of a mod guest checkpoint.
/// The session captures it from its VM; the provider captures the host half.
#[derive(Debug, Clone)]
pub struct ModModuleCheckpoint {
    /// Owning module.
    pub module: ModuleId,
    /// Data allocation bytes.
    pub data: Vec<u8>,
    /// API kind tag.
    pub api_kind: String,
    /// API version.
    pub api_version: u32,
    /// ABI profile.
    pub abi: QvmAbi,
    /// Suspended instruction index.
    pub instruction_index: usize,
    /// Suspended operand stack.
    pub operand_stack: Vec<i32>,
    /// Suspended program stack pointer.
    pub program_stack: usize,
    /// Saved random states.
    pub random_len: usize,
    /// Saved callback count.
    pub callbacks_len: usize,
}

/// Saved mod guest checkpoint, mirroring donor `state.guests[0]`.
#[derive(Debug, Clone)]
pub struct ModGuestCheckpoint {
    /// Module image.
    pub module: ModModuleCheckpoint,
    /// Host-state module.
    pub host_module: ModuleId,
    /// Host-state format tag.
    pub host_format: String,
    /// Host-state value.
    pub host: ProfileValue,
}

/// Saved mod checkpoint, mirroring donor `validateState`/`restore` input.
/// The mods lane decoder feeds this from save bytes; unify post-merge.
#[derive(Debug, Clone, Default)]
pub struct ModCheckpointState {
    /// Guest checkpoints (exactly one QVM guest).
    pub guests: Vec<ModGuestCheckpoint>,
    /// Provider checkpoints (always empty for QVM mods).
    pub providers: Vec<ProviderCheckpoint>,
}

/// VM module-state seam. Donor `QvmModProvider` owns its VM; the guest lane
/// split module ownership to the session host. The session implements this
/// over its VM (`src/compat/qvm/mod-provider.ts` checkpoint path).
pub trait QvmModModuleState {
    /// Capture the module image.
    fn capture_module(&mut self) -> ModModuleCheckpoint;
    /// Restore the module image.
    fn restore_module(&mut self, state: &ModModuleCheckpoint);
    /// Resolve a saved actor.
    fn resolve_actor(&self, saved: SavedActorId) -> Option<ActorId>;
}

/// Mirror of `ModClientPresentationSource` from donor
/// `src/world/session/mod-client-presentation.ts` (canonical home:
/// world/session mod-client-presentation lane); unify post-merge.
#[derive(Clone)]
pub struct ModPresentationSource {
    /// Presentation generation.
    pub generation: Rc<dyn Fn() -> u64>,
    /// Currency assertion.
    pub assert_current: Rc<dyn Fn()>,
    /// Whether the actor has a presentation context.
    pub context: Rc<dyn Fn(&ActorId) -> bool>,
}

impl std::fmt::Debug for ModPresentationSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModPresentationSource").finish_non_exhaustive()
    }
}

/// Client frame view. Donor `view` is always null for QVM mods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModClientFrame {
    /// HUD mode.
    pub hud: QvmModHud,
    /// View (never present).
    pub view: Option<ModClientView>,
}

/// Uninhabited view marker: the donor always reports `view: null`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModClientView {}

/// Client presentation handle, mirroring donor `clientPresentation()`.
#[derive(Clone)]
pub struct ModClientPresentation {
    /// Presentation source.
    pub source: ModPresentationSource,
    /// HUD mode.
    pub hud: QvmModHud,
}

impl std::fmt::Debug for ModClientPresentation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModClientPresentation")
            .field("hud", &self.hud)
            .finish_non_exhaustive()
    }
}

impl ModClientPresentation {
    /// Frame for an actor, or `None` without a presentation context.
    #[must_use]
    pub fn frame(&self, actor: &ActorId) -> Option<ModClientFrame> {
        if !(self.source.context)(actor) {
            return None;
        }
        Some(ModClientFrame {
            hud: self.hud,
            view: None,
        })
    }
}

/// Initialization host, mirroring donor `initialize(context)`.
pub struct QvmModHost<H> {
    /// Destination world services (`None` fails like a null context).
    pub services: Option<H>,
    /// Writable user-file store for mod file mounts.
    pub writable: Option<UserFileStore>,
    /// Cgame presentation source (required when a HUD is declared).
    pub presentation_source: Option<ModPresentationSource>,
    /// Currency assertion.
    pub assert_current: Rc<dyn Fn()>,
    /// Whether the session is restoring (skips initialization).
    pub restoring: bool,
}

/// Owned mount plans. Borrowed plans share sources with their owner and fail
/// once any owner in the chain closes, so the instance retains both.
struct ActiveMounts {
    installed: Option<MountedContent>,
    borrowed: Option<MountedContent>,
}

impl ActiveMounts {
    fn active(&self) -> Option<&MountedContent> {
        self.borrowed.as_ref().or(self.installed.as_ref())
    }
}

/// Initialized QVM gameplay mod instance.
pub struct QvmModInstance<H: ModProviderHost> {
    provider: Rc<RefCell<QvmModProvider<H>>>,
    module: ModuleId,
    declaration: QvmModCallbackDeclaration,
    mounts: ActiveMounts,
    presentation_source: Option<ModPresentationSource>,
    hud: Option<QvmModHud>,
    assert_current: Rc<dyn Fn()>,
}

impl PreparedQvmMod {
    /// Initialize the mod against destination world services.
    pub fn initialize<H: ModProviderHost + 'static>(
        self,
        host: QvmModHost<H>,
    ) -> Result<QvmModInstance<H>, QvmModError> {
        let services = host.services.ok_or(QvmModError::MissingServices)?;
        let mounts = match &host.writable {
            None => ActiveMounts {
                installed: self.mounts,
                borrowed: None,
            },
            Some(writable) => {
                let borrowed = borrow_mod_file_mounts(
                    &self.description.selection,
                    self.description.source.content.clone(),
                    self.mounts.as_ref(),
                    writable,
                )?;
                ActiveMounts {
                    installed: self.mounts,
                    borrowed: Some(borrowed),
                }
            }
        };
        let module = self.provider_artifact.module.clone();
        let mut provider = QvmModProvider::open(self.provider_artifact, self.declaration.clone(), services)?;
        provider.reserve_protection()?;
        if provider.host().has_commands() {
            provider.bind_commands()?;
        }
        if !host.restoring {
            provider.initialize()?;
        }
        (host.assert_current)();
        let hud = self.client_presentation.map(|presentation| presentation.hud);
        if hud.is_some() && host.presentation_source.is_none() {
            return Err(QvmModError::MissingPresentationSource);
        }
        Ok(QvmModInstance {
            provider: Rc::new(RefCell::new(provider)),
            module,
            declaration: self.declaration,
            mounts,
            presentation_source: host.presentation_source,
            hud,
            assert_current: host.assert_current,
        })
    }
}

fn sim_operation(operation: QvmOperation) -> SimOperation {
    match operation {
        QvmOperation::Damage => SimOperation::Damage,
        QvmOperation::InventoryGive => SimOperation::InventoryGive,
        QvmOperation::InventoryConsume => SimOperation::InventoryConsume,
        QvmOperation::ActorThink => SimOperation::ActorThink,
        QvmOperation::ActorTouch => SimOperation::ActorTouch,
        QvmOperation::ActorUse => SimOperation::ActorUse,
        QvmOperation::ActorPain => SimOperation::ActorPain,
        QvmOperation::ActorDie => SimOperation::ActorDie,
    }
}

fn sim_stage(stage: CallbackStage) -> SimStage {
    match stage {
        CallbackStage::Observe => SimStage::Observe,
        CallbackStage::Transform => SimStage::Transform,
        CallbackStage::Replace => SimStage::Replace,
    }
}

/// Map a result selector. Only damage transforms read `result`, so absent
/// and boolean selectors default to the amount field.
fn sim_result(result: Option<CallbackResult>) -> DamageField {
    match result {
        Some(CallbackResult::Knockback) => DamageField::Knockback,
        Some(CallbackResult::Amount) | Some(CallbackResult::Boolean) | None => DamageField::Amount,
    }
}

fn sim_callback(binding: &ModCallbackBinding) -> SimCallback {
    SimCallback {
        id: binding.id.clone(),
        operation: sim_operation(binding.operation),
        stage: sim_stage(binding.stage),
        result: sim_result(binding.result),
    }
}

fn qvm_input(input: SimInput) -> QvmInput {
    match input {
        SimInput::Time => QvmInput::Time,
        SimInput::Result => QvmInput::Result,
        SimInput::SelfActor => QvmInput::Own,
        SimInput::Attacker => QvmInput::Attacker,
        SimInput::Inflictor => QvmInput::Inflictor,
        SimInput::Amount => QvmInput::Amount,
        SimInput::Knockback => QvmInput::Knockback,
        SimInput::Direction => QvmInput::Direction,
        SimInput::Point => QvmInput::Point,
        SimInput::Normal => QvmInput::Normal,
        SimInput::Item => QvmInput::Item,
        SimInput::Other => QvmInput::Other,
        SimInput::Activator => QvmInput::Activator,
        SimInput::Elapsed => QvmInput::Elapsed,
    }
}

fn qvm_value(value: &SimValue) -> QvmValue {
    match value {
        SimValue::Float(value) => QvmValue::Float(*value),
        SimValue::Actor(actor) => QvmValue::Actor(actor.clone()),
        SimValue::Vector(vector) => QvmValue::Vec(Vec3 {
            x: vector[0] as f32,
            y: vector[1] as f32,
            z: vector[2] as f32,
        }),
        SimValue::Text(text) => QvmValue::Str(text.clone()),
    }
}

impl<H: ModProviderHost> std::fmt::Debug for QvmModInstance<H> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QvmModInstance").finish_non_exhaustive()
    }
}

impl<H: ModProviderHost + 'static> QvmModInstance<H> {
    /// Activate mod protection.
    pub fn activate(&mut self) -> Result<(), qa_guest::error::GuestError> {
        self.provider.borrow_mut().activate_protection()
    }

    /// Register behavior callbacks.
    pub fn register(&self, registrations: &mut dyn ModRegistrations) {
        let provider = Rc::clone(&self.provider);
        let declaration = self.declaration.clone();
        let assert_current = Rc::clone(&self.assert_current);
        let time_provider = Rc::clone(&self.provider);
        let time: Rc<dyn Fn() -> SourceTime> =
            Rc::new(move || SourceTime::Seconds(time_provider.borrow().host().time_seconds() as f32));
        let execute: super::mod_callbacks::ModCallbackExecute = Rc::new(move |callback, inputs| {
            assert_current();
            let found = declaration
                .callbacks
                .iter()
                .find(|declared| declared.binding.id == callback.id)?;
            let mapped: BTreeMap<QvmInput, QvmValue> = inputs
                .iter()
                .map(|(input, value)| (qvm_input(*input), qvm_value(value)))
                .collect();
            match provider.borrow_mut().invoke(&found.call, &mapped) {
                Ok(value) => Some(value),
                Err(error) => panic!("QVM mod callback failed: {error}"),
            }
        });
        let callbacks: Vec<SimCallback> = self
            .declaration
            .callbacks
            .iter()
            .map(|callback| sim_callback(&callback.binding))
            .collect();
        register_mod_callbacks(&callbacks, registrations, time, execute);
    }

    /// Advance one frame.
    pub fn advance(&mut self, frame: &FrameContext) -> Result<(), qa_guest::error::GuestError> {
        self.provider
            .borrow_mut()
            .advance(frame.time.as_seconds_f64(), frame.elapsed.as_seconds_f64())
    }

    /// Owned actor presentations.
    #[must_use]
    pub fn presentations(&self) -> Vec<ModPresentation> {
        self.provider.borrow().presentations()
    }

    /// Capture a mod checkpoint.
    pub fn checkpoint(&mut self, module: &mut dyn QvmModModuleState) -> Result<ModCheckpointState, QvmModError> {
        let host = self.provider.borrow_mut().capture_host_state()?;
        Ok(ModCheckpointState {
            guests: vec![ModGuestCheckpoint {
                module: module.capture_module(),
                host_module: self.module.clone(),
                host_format: "qvm:mod-host-v1".to_string(),
                host,
            }],
            providers: Vec::new(),
        })
    }

    /// Restore a mod checkpoint.
    pub fn restore(
        &mut self,
        state: &ModCheckpointState,
        module: &mut dyn QvmModModuleState,
    ) -> Result<(), QvmModError> {
        let Some(guest) = state.guests.first() else {
            return Err(QvmModError::MissingCheckpoint);
        };
        module.restore_module(&guest.module);
        let resolve = |saved: SavedActorId| module.resolve_actor(saved);
        self.provider
            .borrow_mut()
            .restore_host_state(&guest.host, &guest.module.data, &resolve)?;
        Ok(())
    }

    /// Presentation source for the cgame runtime.
    #[must_use]
    pub fn qvm_presentation(&self) -> Option<&ModPresentationSource> {
        self.presentation_source.as_ref()
    }

    /// Client presentation handle.
    #[must_use]
    pub fn client_presentation(&self) -> Option<ModClientPresentation> {
        let source = self.presentation_source.clone()?;
        let hud = self.hud?;
        Some(ModClientPresentation { source, hud })
    }

    /// Active content mounts (the borrowed mod files, if any).
    #[must_use]
    pub fn mounts(&self) -> Option<&MountedContent> {
        self.mounts.active()
    }

    /// Close the mod.
    pub fn close(&mut self) -> Result<(), qa_guest::error::GuestError> {
        self.provider.borrow_mut().close()
    }
}

#[cfg(test)]
mod tests {
    use super::super::mod_callbacks::DamageOutcome as SimOutcome;
    use super::super::mod_callbacks::DamageRequest as SimDamage;
    use super::super::mod_callbacks::ModCallbackHandler;
    use super::super::mod_callbacks::ModRegistrationTarget;
    use super::*;
    use qa_content::contract::create_mount_id;
    use qa_content::contract::create_mount_identity;
    use qa_content::contract::create_mount_plan_id;
    use qa_content::contract::ContentMount;
    use qa_content::contract::LooseMount;
    use qa_content::contract::ModAvailability;
    use qa_content::contract::ModPurpose;
    use qa_content::contract::ModSelection;
    use qa_content::contract::ProviderReference;
    use qa_content::contract::ResolvedMountPlan;
    use qa_content::mounts::digest_bytes;
    use qa_content::mounts::open_mount_plan;
    use qa_content::mounts::OpenMountOptions;
    use qa_core::identity::IdentityOwner;
    use qa_core::identity::ProviderId;
    use qa_core::time::FramePhase;
    use qa_guest::error::GuestError;
    use qa_guest::qvm::artifacts::sha256_hex;
    use qa_guest::qvm::mod_presentation::PlayerEventCentities;
    use qa_guest::qvm::mod_presentation::PlayerEventStorage;
    use qa_guest::qvm::mod_presentation::PresentationHud;
    use qa_guest::qvm::mod_presentation::QvmPlayerEventPresentation;
    use qa_guest::qvm::mod_presentation::QvmPresentationCall;
    use qa_guest::qvm::mod_presentation::QvmPresentationProgram;
    use qa_guest::qvm::mod_presentation_checkpoint::SourceGameState;
    use qa_guest::qvm::mod_provider::CanonicalField;
    use qa_guest::qvm::mod_provider::EntityLinkView;
    use qa_guest::qvm::mod_provider::ModReturns;
    use qa_guest::qvm::mod_provider::QvmModActorField;
    use qa_guest::qvm::mod_provider::QvmModCallback;
    use qa_guest::qvm::mod_provider::QvmModSourceCall;

    fn qvm_bytes(data_length: usize) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0x1272_1444u32.to_le_bytes());
        bytes.extend_from_slice(&1i32.to_le_bytes());
        bytes.extend_from_slice(&32i32.to_le_bytes());
        bytes.extend_from_slice(&5i32.to_le_bytes());
        bytes.extend_from_slice(&40i32.to_le_bytes());
        bytes.extend_from_slice(&(data_length as i32).to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&[3, 8, 0, 0, 0]);
        bytes.extend_from_slice(&[0, 0, 0]);
        bytes.extend(vec![0; data_length]);
        bytes
    }

    fn description() -> ModDescription {
        ModDescription {
            selection: ModSelection {
                product: "q3".to_string(),
                id: "test".to_string(),
            },
            source: ProviderReference {
                provider: ProviderId::new("test", "mod"),
                content: qa_content::contract::ContentId("q3:classic:test:v1".to_string()),
            },
            title: "Test".to_string(),
            source_title: "Test".to_string(),
            purpose: ModPurpose::GameType,
            requires: Vec::new(),
            conflicts: Vec::new(),
            availability: ModAvailability::Available,
        }
    }

    fn declaration() -> QvmModCallbackDeclaration {
        QvmModCallbackDeclaration {
            version: 1,
            program_path: "vm/qagame.qvm".to_string(),
            program_digest: "sha256:abc".to_string(),
            abi_profile: QvmAbi::Modern,
            presentation: None,
            spawn_entities: None,
            clients: None,
            actor_records: Vec::new(),
            entity_record: None,
            source_actors: None,
            combat: None,
            protection: Vec::new(),
            pickups: Vec::new(),
            items: None,
            initialize: Vec::new(),
            callbacks: Vec::new(),
            objectives: Vec::new(),
        }
    }

    fn presentation_declaration(gameplay_digest: &str, cgame_digest: &str) -> QvmModPresentationDeclaration {
        QvmModPresentationDeclaration::PlayerEvents(QvmPlayerEventPresentation {
            gameplay: QvmPresentationProgram {
                path: "vm/qagame.qvm".to_string(),
                digest: gameplay_digest.to_string(),
                abi: QvmAbi::Modern,
            },
            cgame: QvmPresentationProgram {
                path: "vm/cgame.qvm".to_string(),
                digest: cgame_digest.to_string(),
                abi: QvmAbi::Modern,
            },
            initialize: Vec::new(),
            refresh: Vec::new(),
            frame: Vec::new(),
            hud: Some(PresentationHud {
                mode: HudMode::Overlay,
                frame: Vec::new(),
            }),
            storage: PlayerEventStorage {
                game_state: 0,
                player_state: 73872,
                snapshot_address: 20100,
                snapshot_pointers: Vec::new(),
                centities: PlayerEventCentities {
                    address: 0,
                    stride: 512,
                    capacity: 1,
                    state: 0,
                    origin: 12,
                },
                time: Vec::new(),
                frame_time: Vec::new(),
                view_origin: Vec::new(),
                view_angles: Vec::new(),
                view_axis: Vec::new(),
            },
            project: Vec::new(),
            event: QvmPresentationCall {
                entry: 0,
                when_weapon_presented: false,
                arguments: Vec::new(),
            },
        })
    }

    fn options() -> PrepareQvmModOptions {
        let program = qvm_bytes(64);
        let mut decl = declaration();
        decl.program_digest = format!("sha256:{}", sha256_hex(&program));
        PrepareQvmModOptions {
            description: description(),
            declaration: decl,
            declaration_digest: ContentDigest("sha256:decl".to_string()),
            program,
            presentation_program: None,
            mounts: None,
        }
    }

    struct StubHost {
        commands: bool,
    }

    impl ModProviderHost for StubHost {
        fn current(&self) -> Result<(), GuestError> {
            Ok(())
        }
        fn is_live(&self, _actor: &ActorId) -> bool {
            true
        }
        fn is_owned(&self, _actor: &ActorId) -> bool {
            false
        }
        fn time_seconds(&self) -> f64 {
            1.5
        }
        fn call_module(&mut self, _words: &[i32], _entry: usize) -> Result<i32, GuestError> {
            Ok(0)
        }
        fn command_module(&mut self, _words: &[i32], _argv: &[String]) -> Result<i32, GuestError> {
            Ok(0)
        }
        fn read_i32(&self, _address: usize) -> Result<i32, GuestError> {
            Ok(0)
        }
        fn write_i32(&mut self, _address: usize, _value: i32) -> Result<(), GuestError> {
            Ok(())
        }
        fn read_bytes(&self, _address: usize, len: usize) -> Result<Vec<u8>, GuestError> {
            Ok(vec![0; len])
        }
        fn write_bytes(&mut self, _address: usize, _bytes: &[u8]) -> Result<(), GuestError> {
            Ok(())
        }
        fn copy_bytes(&mut self, _dest: usize, _src: usize, _len: usize) -> Result<(), GuestError> {
            Ok(())
        }
        fn stack_pointer(&self) -> usize {
            0
        }
        fn canonical_field(&self, _actor: &ActorId, _field: &QvmModActorField) -> Result<CanonicalField, GuestError> {
            Ok(CanonicalField::Word(0.0))
        }
        fn commit_field(
            &mut self,
            _actor: &ActorId,
            _field: &QvmModActorField,
            _value: CanonicalField,
        ) -> Result<(), GuestError> {
            Ok(())
        }
        fn has_client(&self, _actor: &ActorId) -> bool {
            false
        }
        fn client_slot(&self, _actor: &ActorId) -> Option<usize> {
            None
        }
        fn admitted_client(&self, _actor: &ActorId) -> bool {
            false
        }
        fn players(&self) -> Vec<(ActorId, usize, bool)> {
            Vec::new()
        }
        fn start_clients(&mut self) -> Result<(), GuestError> {
            Ok(())
        }
        fn frame_actors(&self) -> Vec<ActorId> {
            Vec::new()
        }
        fn release_actor_components(&mut self, _actor: &ActorId) {}
        fn reserve_protection(&mut self) -> Result<(), GuestError> {
            Ok(())
        }
        fn activate_protection(&mut self) -> Result<(), GuestError> {
            Ok(())
        }
        fn assert_subcomponents_idle(&self) -> Result<(), GuestError> {
            Ok(())
        }
        fn close_subcomponents(&mut self) -> Vec<GuestError> {
            Vec::new()
        }
        fn publish_player_events(&mut self) {}
        fn discard_player_events(&mut self) {}
        fn check_pickup_write(&self, _actor: &ActorId, _field: &QvmModActorField) -> Result<(), GuestError> {
            Ok(())
        }
        fn adopt_source(&mut self, _slot: usize) -> Result<ActorId, GuestError> {
            Err(GuestError::invalid("no source actors"))
        }
        fn retire_source(&mut self, _slot: usize) -> Result<(), GuestError> {
            Ok(())
        }
        fn before_release(&mut self, _actor: &ActorId) -> Result<(), GuestError> {
            Ok(())
        }
        fn release_owned(&mut self, _actor: &ActorId) -> Result<(), GuestError> {
            Ok(())
        }
        fn begin_actor_frame(&mut self) -> Result<(), GuestError> {
            Ok(())
        }
        fn end_actor_frame(&mut self) -> Result<bool, GuestError> {
            Ok(false)
        }
        fn server_info(&self) -> (String, String) {
            (String::new(), String::new())
        }
        fn build_game_state(&self, _entries: &[(u32, String)]) -> Result<SourceGameState, GuestError> {
            Ok(SourceGameState {
                offsets: Vec::new(),
                data: Vec::new(),
                count: 0,
            })
        }
        fn owned_entity_views(&self) -> Vec<qa_guest::qvm::mod_provider::EntityPublishView> {
            Vec::new()
        }
        fn player_state_bytes(&self, _actor: &ActorId) -> Result<Vec<u8>, GuestError> {
            Ok(Vec::new())
        }
        fn entity_link(&self, _slot: usize) -> Result<EntityLinkView, GuestError> {
            Ok(EntityLinkView {
                linked: false,
                sv_flags: 0,
                single_client: -1,
                abs_min: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                abs_max: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            })
        }
        fn emit(&mut self, _event: qa_guest::qvm::mod_provider::ProviderEmit) -> Result<(), GuestError> {
            Ok(())
        }
        fn read_script(&self, _name: &str) -> Option<String> {
            None
        }
        fn has_commands(&self) -> bool {
            self.commands
        }
    }

    struct StubModule {
        image: ModModuleCheckpoint,
    }

    impl QvmModModuleState for StubModule {
        fn capture_module(&mut self) -> ModModuleCheckpoint {
            self.image.clone()
        }
        fn restore_module(&mut self, state: &ModModuleCheckpoint) {
            self.image = state.clone();
        }
        fn resolve_actor(&self, _saved: SavedActorId) -> Option<ActorId> {
            None
        }
    }

    struct StubRegistrations {
        handlers: Vec<(String, ModCallbackHandler)>,
    }

    impl ModRegistrations for StubRegistrations {
        fn register(&mut self, id: &str, _target: ModRegistrationTarget, handler: ModCallbackHandler) {
            self.handlers.push((id.to_string(), handler));
        }
    }

    fn host() -> QvmModHost<StubHost> {
        QvmModHost {
            services: Some(StubHost { commands: false }),
            writable: None,
            presentation_source: None,
            assert_current: Rc::new(|| {}),
            restoring: false,
        }
    }

    #[test]
    fn prepare_rejects_bad_digest() {
        let mut opts = options();
        opts.declaration.program_digest = "bogus".to_string();
        assert!(matches!(prepare_qvm_mod(opts).unwrap_err(), QvmModError::BadDigest));
    }

    #[test]
    fn prepare_validates_minimal_mod() {
        let prepared = prepare_qvm_mod(options()).unwrap();
        assert_eq!(prepared.identity.modules.len(), 1);
        assert_eq!(prepared.identity.modules[0].id.namespace, "mod");
        assert_eq!(prepared.identity.modules[0].artifact_path, "vm/qagame.qvm");
        assert!(prepared.presentation.is_none());
        assert!(prepared.client_presentation.is_none());
        assert!(prepared.hub_presentation().is_none());
    }

    #[test]
    fn prepare_assembles_presentation() {
        let mut opts = options();
        let cgame = qvm_bytes(74340);
        let digest = format!("sha256:{}", sha256_hex(&cgame));
        let gameplay = opts.declaration.program_digest.clone();
        opts.declaration.presentation = Some(presentation_declaration(&gameplay, &digest));
        opts.presentation_program = Some(cgame);
        let prepared = prepare_qvm_mod(opts).unwrap();
        let presentation = prepared.presentation.as_ref().unwrap();
        assert_eq!(presentation.source.artifact_path, "vm/qagame.qvm");
        assert_eq!(
            prepared.client_presentation,
            Some(QvmModClientPresentation {
                hud: QvmModHud::Overlay,
                view: false,
            })
        );
        assert!(prepared.hub_presentation().is_some());
    }

    #[test]
    fn prepare_rejects_presentation_mismatch() {
        let mut missing = options();
        let cgame = qvm_bytes(74340);
        let digest = format!("sha256:{}", sha256_hex(&cgame));
        let gameplay = missing.declaration.program_digest.clone();
        missing.declaration.presentation = Some(presentation_declaration(&gameplay, &digest));
        assert!(matches!(
            prepare_qvm_mod(missing).unwrap_err(),
            QvmModError::MissingPresentationBytes
        ));
        let mut extra = options();
        extra.presentation_program = Some(qvm_bytes(64));
        assert!(matches!(
            prepare_qvm_mod(extra).unwrap_err(),
            QvmModError::PresentationBytesWithoutDeclaration
        ));
        let mut skewed = options();
        let cgame = qvm_bytes(74340);
        let digest = format!("sha256:{}", sha256_hex(&cgame));
        let gameplay = skewed.declaration.program_digest.clone();
        let mut presentation = presentation_declaration(&gameplay, &digest);
        let QvmModPresentationDeclaration::PlayerEvents(inner) = &mut presentation else {
            panic!("fixture is player events");
        };
        inner.gameplay.abi = QvmAbi::Legacy;
        skewed.declaration.presentation = Some(presentation);
        skewed.presentation_program = Some(qvm_bytes(74340));
        assert!(matches!(prepare_qvm_mod(skewed).unwrap_err(), QvmModError::AbiMismatch));
    }

    fn loose_mounts(dir: &std::path::Path) -> MountedContent {
        let mount_id = create_mount_id("test", "loose").unwrap();
        let plan = ResolvedMountPlan {
            id: create_mount_plan_id("test", "r1").unwrap(),
            mounts: vec![ContentMount::Loose(LooseMount {
                identity: create_mount_identity(
                    mount_id.clone(),
                    qa_content::contract::ContentId("q3:classic:test:v1".to_string()),
                    0,
                )
                .unwrap(),
                root_path: dir.to_string_lossy().into_owned(),
            })],
            default_order: vec![mount_id],
            prefix_orders: Vec::new(),
        };
        open_mount_plan(&plan, OpenMountOptions::default()).unwrap()
    }

    #[test]
    fn prepare_mounted_reads_programs() {
        let dir = std::env::temp_dir().join("qvm-mod-test-mounted");
        std::fs::create_dir_all(dir.join("vm")).unwrap();
        let bytes = qvm_bytes(64);
        std::fs::write(dir.join("vm").join("qagame.qvm"), &bytes).unwrap();
        let mut decl = declaration();
        decl.program_digest = digest_bytes(&bytes).as_str().to_string();
        let prepared = prepare_mounted_qvm_mod(PrepareMountedQvmModOptions {
            description: description(),
            declaration: decl,
            declaration_digest: ContentDigest("sha256:decl".to_string()),
            mounts: loose_mounts(&dir),
        })
        .unwrap();
        assert_eq!(prepared.identity.modules[0].artifact_path, "vm/qagame.qvm");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn prepare_mounted_rejects_drift() {
        let dir = std::env::temp_dir().join("qvm-mod-test-drift");
        std::fs::create_dir_all(dir.join("vm")).unwrap();
        std::fs::write(dir.join("vm").join("qagame.qvm"), qvm_bytes(64)).unwrap();
        let mut decl = declaration();
        decl.program_digest = "sha256:deadbeef".to_string();
        let error = prepare_mounted_qvm_mod(PrepareMountedQvmModOptions {
            description: description(),
            declaration: decl,
            declaration_digest: ContentDigest("sha256:decl".to_string()),
            mounts: loose_mounts(&dir),
        })
        .unwrap_err();
        assert!(matches!(error, QvmModError::SelectedDiffers));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn initialize_requires_services() {
        let prepared = prepare_qvm_mod(options()).unwrap();
        let mut host = host();
        host.services = None;
        assert!(matches!(
            prepared.initialize(host).unwrap_err(),
            QvmModError::MissingServices
        ));
    }

    #[test]
    fn initialize_runs_lifecycle() {
        let prepared = prepare_qvm_mod(options()).unwrap();
        let mut instance = prepared.initialize(host()).unwrap();
        instance.activate().unwrap();
        let frame = FrameContext {
            frame: 1,
            time: SourceTime::Seconds(1.0),
            elapsed: SourceTime::Seconds(0.05),
            phase: FramePhase::FrameEntry,
        };
        instance.advance(&frame).unwrap();
        assert!(instance.presentations().is_empty());
        assert!(instance.mounts().is_none());
        assert!(instance.qvm_presentation().is_none());
        assert!(instance.client_presentation().is_none());
        instance.close().unwrap();
    }

    #[test]
    fn register_drives_callbacks() {
        let owner = IdentityOwner::create("register").unwrap();
        let mut opts = options();
        opts.declaration.callbacks = vec![QvmModCallback {
            binding: ModCallbackBinding {
                id: "dmg".to_string(),
                operation: QvmOperation::Damage,
                stage: CallbackStage::Observe,
                result: None,
            },
            call: QvmModSourceCall {
                entry: 0,
                arguments: Vec::new(),
                globals: Vec::new(),
                returns: ModReturns::Float32,
            },
        }];
        let instance = prepare_qvm_mod(opts).unwrap().initialize(host()).unwrap();
        let mut registrations = StubRegistrations { handlers: Vec::new() };
        instance.register(&mut registrations);
        assert_eq!(registrations.handlers.len(), 1);
        assert_eq!(registrations.handlers[0].0, "dmg");
        let actor = owner.actor(1, 0);
        let request = SimDamage {
            target: Some(actor),
            attacker: None,
            inflictor: None,
            amount: 10.0,
            knockback: 0.0,
            direction: [0.0, 0.0, 1.0],
            point: [0.0, 0.0, 0.0],
            normal: [0.0, 0.0, 1.0],
        };
        let outcome = SimOutcome::Committed { applied_damage: 10.0 };
        let ModCallbackHandler::DamageObserve(observe) = &registrations.handlers[0].1 else {
            panic!("damage observe handler");
        };
        observe(&request, &outcome);
    }

    fn module_image(prepared: &PreparedQvmMod) -> ModModuleCheckpoint {
        ModModuleCheckpoint {
            module: module_id(&prepared.module),
            data: vec![0; 64],
            api_kind: "q3-qagame".to_string(),
            api_version: 8,
            abi: QvmAbi::Modern,
            instruction_index: 0,
            operand_stack: Vec::new(),
            program_stack: 64,
            random_len: 0,
            callbacks_len: 0,
        }
    }

    #[test]
    fn checkpoint_restore_roundtrip_validates() {
        let prepared = prepare_qvm_mod(options()).unwrap();
        let mut module = StubModule {
            image: module_image(&prepared),
        };
        let mut instance = prepared.initialize(host()).unwrap();
        let checkpoint = instance.checkpoint(&mut module).unwrap();
        assert_eq!(checkpoint.guests.len(), 1);
        assert_eq!(checkpoint.guests[0].host_format, "qvm:mod-host-v1");
        assert!(checkpoint.providers.is_empty());
        instance.restore(&checkpoint, &mut module).unwrap();
        assert!(matches!(
            instance
                .restore(&ModCheckpointState::default(), &mut module)
                .unwrap_err(),
            QvmModError::MissingCheckpoint
        ));
    }

    #[test]
    fn validate_state_checks_shape() {
        let prepared = prepare_qvm_mod(options()).unwrap();
        assert!(matches!(
            prepared.validate_state(&ModCheckpointState::default()).unwrap_err(),
            QvmModError::MissingCheckpoint
        ));
    }

    #[test]
    fn client_presentation_frames_context() {
        let owner = IdentityOwner::create("frame").unwrap();
        let mut opts = options();
        let cgame = qvm_bytes(74340);
        let digest = format!("sha256:{}", sha256_hex(&cgame));
        let gameplay = opts.declaration.program_digest.clone();
        opts.declaration.presentation = Some(presentation_declaration(&gameplay, &digest));
        opts.presentation_program = Some(cgame);
        let prepared = prepare_qvm_mod(opts).unwrap();
        let seva = owner.actor(1, 0);
        let probe = seva.clone();
        let other = owner.actor(2, 0);
        let mut host = host();
        host.presentation_source = Some(ModPresentationSource {
            generation: Rc::new(|| 7),
            assert_current: Rc::new(|| {}),
            context: Rc::new(move |actor: &ActorId| *actor == probe),
        });
        let instance = prepared.initialize(host).unwrap();
        let presentation = instance.client_presentation().unwrap();
        assert_eq!(presentation.hud, QvmModHud::Overlay);
        let frame = presentation.frame(&seva).unwrap();
        assert_eq!(frame.hud, QvmModHud::Overlay);
        assert_eq!(frame.view, None);
        assert!(presentation.frame(&other).is_none());
        assert!((instance.qvm_presentation().unwrap().generation)() == 7);
    }

    #[test]
    fn mappings_cover_callback_shapes() {
        assert_eq!(sim_operation(QvmOperation::ActorDie), SimOperation::ActorDie);
        assert_eq!(sim_stage(CallbackStage::Replace), SimStage::Replace);
        assert_eq!(sim_result(Some(CallbackResult::Knockback)), DamageField::Knockback);
        assert_eq!(sim_result(Some(CallbackResult::Boolean)), DamageField::Amount);
        assert_eq!(sim_result(None), DamageField::Amount);
        assert_eq!(qvm_input(SimInput::SelfActor), QvmInput::Own);
        assert_eq!(qvm_input(SimInput::Elapsed), QvmInput::Elapsed);
        assert_eq!(qvm_value(&SimValue::Float(2.0)), QvmValue::Float(2.0));
        assert!(matches!(qvm_value(&SimValue::Text("x".to_string())), QvmValue::Str(_)));
    }
}
