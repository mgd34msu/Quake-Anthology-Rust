//! Native gameplay mod prepare + lifecycle.
//!
//! Port of donor `src/app/bootstrap/simulation/native-mod.ts`
//! (`prepareNativeMod`, `prepareMountedNativeMod`).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use qa_compat::q2::native_mod_provider::{
    ApiKind, CallAccepts, CallReturns, ModAddress, ModCallDecl, ModEntryRef, ModGlobal, ModValueKind, NativeActorId,
    NativeModDeclaration as ProviderDeclaration, NativeModProvider, ProviderError, ProviderHost, ProviderServices,
    RuntimeValue,
};
use qa_content::contract::{
    ContentDigest, ModDescription, ModIdentity, ModuleIdentity, NativeModAddress, NativeModCallback,
    NativeModDeclaration, NativeModEntry, NativeModGlobal, NativeModReturn, NativeModScalar, NativeModSourceCall,
    NativeModTarget, NativeModValue, ProviderCheckpointHeader, ProviderReference, ResolvedResourceReference,
};
use qa_content::hash::sha256_hex;
use qa_content::mounts::MountedContent;
use qa_core::identity::{ActorId, IdentityOwner, OwnedActor, ProviderId};
use qa_guest::checkpoint::{GameApi, NativeCall, NativeCallAbi};
use qa_guest::core::contracts::{GuestAddress, GuestCallResult, GuestCallValue, GuestStorage};
use qa_guest::pe::format::parse_pe;

use super::classic_guest_source::PreparedClassicGuest;
use super::native_mod_host::{
    BindCommandsFn, ModResourceServices, NativeModCvarCaptureFn, NativeModCvarRestoreFn, NativeModHost,
    NativeModHostContext, NativeModHostOptions, NativeModPrepared, NativeModProjection, SharedProjection,
};
use super::native_mod_presentation::{ModEdition, NativeModPresentationCheckpoint};
use super::rerelease_guest_source::PreparedRereleaseGuest;
use super::types::{GuestLocalize, ModClientPresentationAdmission, ModModuleCheckpoint, PreparedMod};
use crate::persistence::recipe::{ExecutionImplementation, ResolvedExecutionModule};
use qa_compat::q2::native_primary::PrimaryEdition;

/// Native mod error.
#[derive(Debug, thiserror::Error)]
pub enum NativeModError {
    /// Invalid mod state or input.
    #[error("invalid native mod: {0}")]
    Invalid(String),
}

impl NativeModError {
    /// Invalid-data error.
    #[must_use]
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }
}

fn mapped(error: impl ToString) -> NativeModError {
    NativeModError::invalid(error.to_string())
}

fn provider_mapped(error: ProviderError) -> NativeModError {
    NativeModError::invalid(format!("Native mod provider failed: {error}"))
}

/// Script reader seam (donor `readScript`).
pub type ReadScriptFn = Rc<dyn Fn(&str) -> Option<String>>;

/// Provider-declaration converter seam: the content declaration and the
/// compat provider declaration are different authoring formats (record
/// bindings, acceptance rules, and call shapes have no mechanical
/// mapping); the owning lane supplies the converter. Cites donor
/// `src/contracts/native-mod-callbacks.ts` and
/// `src/compat/q2/native-mod-provider.ts`.
pub type ConvertDeclarationFn = Rc<dyn Fn(&NativeModDeclaration) -> Result<ProviderDeclaration, NativeModError>>;

/// Session-current guard seam (donor `context.assertCurrent`).
pub type AssertCurrentFn = Rc<dyn Fn()>;

/// Mod callback invoker: runtime inputs to the guest decision.
pub type ModCallbackInvoker = Rc<dyn Fn(HashMap<String, RuntimeValue>) -> Result<Option<i32>, NativeModError>>;

/// Session callback table seam (donor `registerModCallbacks` from
/// `./mod-callbacks.ts`, home: mod-callbacks lane; unify post-merge).
/// The instance wires real invokers; the session owns the table.
pub trait ModCallbackSink {
    /// Register one callback invoker.
    fn add(&mut self, callback: &NativeModCallback, invoke: ModCallbackInvoker);
}

/// Prepare options (donor `PrepareNativeModOptions`).
pub struct PrepareNativeModOptions {
    /// Mod description.
    pub description: ModDescription,
    /// Mod declaration.
    pub declaration: NativeModDeclaration,
    /// Declaration digest.
    pub declaration_digest: ContentDigest,
    /// Program bytes.
    pub program: Vec<u8>,
    /// Resolved artifact.
    pub artifact: ResolvedResourceReference,
    /// Script reader.
    pub read_script: Option<ReadScriptFn>,
    /// Localizer.
    pub localize: GuestLocalize,
}

/// Mounted prepare options (donor `prepareMountedNativeMod`).
pub struct PrepareMountedNativeModOptions {
    /// Mod description.
    pub description: ModDescription,
    /// Mod declaration.
    pub declaration: NativeModDeclaration,
    /// Declaration digest.
    pub declaration_digest: ContentDigest,
    /// Content mounts (consumed for lazy script reads).
    pub mounts: MountedContent,
}

/// Prepared native mod: session-lane data plus host-ready pieces.
pub struct NativePreparedMod {
    /// Session data.
    pub data: PreparedMod,
    /// Prepared guest.
    pub host: NativeModPrepared,
    /// Mod declaration.
    pub declaration: NativeModDeclaration,
    /// Mod source identity.
    pub source: ProviderReference,
    /// Localizer.
    pub localize: GuestLocalize,
    /// Script reader, for the session's command binder.
    pub read_script: Option<ReadScriptFn>,
}

/// Saved native mod state (donor validateState input).
pub struct NativeModSavedState {
    /// Guest records.
    pub guests: usize,
    /// Provider records.
    pub providers: Vec<qa_compat::q2::native_mod_provider::ProviderCheckpoint>,
}

/// Lower a content artifact to the recipe record.
fn convert_artifact(artifact: &ResolvedResourceReference) -> crate::persistence::recipe::ResolvedResourceReference {
    use crate::persistence::recipe;
    let provenance = match &artifact.provenance {
        qa_content::contract::ResourceProvenance::Archive {
            mount,
            member_path,
            member_index,
        } => recipe::ResourceProvenance::Archive {
            mount: Box::new(recipe::ContentMount::Archive {
                identity: recipe::MountIdentity {
                    id: mount.identity.id.as_str().to_string(),
                    content: mount.identity.content.as_str().to_string(),
                    generation: mount.identity.generation,
                },
                format: match mount.format {
                    qa_content::contract::ArchiveFormat::Pak => "pak".to_string(),
                    qa_content::contract::ArchiveFormat::Pk3 => "pk3".to_string(),
                    qa_content::contract::ArchiveFormat::Kpf => "kpf".to_string(),
                    qa_content::contract::ArchiveFormat::Zip => "zip".to_string(),
                },
                archive_path: mount.archive_path.clone(),
                archive_digest: mount.archive_digest.as_str().to_string(),
            }),
            member_path: member_path.clone(),
            member_index: *member_index,
        },
        qa_content::contract::ResourceProvenance::Loose { mount, member_path } => recipe::ResourceProvenance::Loose {
            mount: Box::new(recipe::ContentMount::Loose {
                identity: recipe::MountIdentity {
                    id: mount.identity.id.as_str().to_string(),
                    content: mount.identity.content.as_str().to_string(),
                    generation: mount.identity.generation,
                },
                root_path: mount.root_path.clone(),
            }),
            member_path: member_path.clone(),
        },
    };
    let resolution = match &artifact.resolution {
        qa_content::contract::ResourceResolution::DefaultOrder { plan, rank } => {
            recipe::ResourceResolution::DefaultOrder {
                plan: plan.as_str().to_string(),
                rank: *rank,
            }
        }
        qa_content::contract::ResourceResolution::PrefixOrder { plan, prefix, rank } => {
            recipe::ResourceResolution::PrefixOrder {
                plan: plan.as_str().to_string(),
                prefix: prefix.clone(),
                rank: *rank,
            }
        }
        qa_content::contract::ResourceResolution::Link {
            plan,
            source_prefix,
            target_path,
        } => recipe::ResourceResolution::Link {
            plan: plan.as_str().to_string(),
            source_prefix: source_prefix.clone(),
            target_path: target_path.clone(),
        },
    };
    recipe::ResolvedResourceReference {
        id: artifact.id.as_str().to_string(),
        requested_path: artifact.requested_path.clone(),
        provenance,
        digest: artifact.digest.as_str().to_string(),
        byte_length: artifact.byte_length,
        resolution,
    }
}

fn artifact_mismatch() -> NativeModError {
    NativeModError::invalid("Native mod artifact or ABI differs from its declaration")
}

/// Prepare a native mod from mounted content.
pub fn prepare_mounted_native_mod(
    options: PrepareMountedNativeModOptions,
) -> Result<NativePreparedMod, NativeModError> {
    let resource = options
        .mounts
        .open(&options.declaration.program.path, |_| true)
        .map_err(mapped)?
        .filter(|resource| resource.reference.digest == options.declaration.program.digest)
        .ok_or_else(|| NativeModError::invalid("Selected native mod differs from its resolved artifact"))?;
    let mounts = Rc::new(options.mounts);
    let mut reader = MountReader {
        mounts: Rc::clone(&mounts),
    };
    let localization = qa_client::text::resources::load_server_localization_resources(
        "english",
        &mut reader,
        qa_client::text::localization::LocalizationProfile::Q2Rerelease,
    );
    let table = Rc::new(localization);
    let localize: GuestLocalize = Rc::new(move |key, args| {
        // No `Q2LocalizationCatalog` implementation exists for
        // `LocalizationTable` in-worktree, so the table's native lookup
        // serves; the q2-specific catalog path unifies post-merge.
        table.lookup(key, args).unwrap_or_else(|| key.to_string())
    });
    let scripts = Rc::clone(&mounts);
    let read_script: ReadScriptFn = Rc::new(move |name| {
        scripts
            .open(name, |_| true)
            .ok()
            .flatten()
            .map(|resource| String::from_utf8_lossy(&resource.bytes).into_owned())
    });
    // Keep mounts alive through the returned closures is handled by the
    // `Rc` clones above; `mounts` itself is dropped here.
    let _ = mounts;
    prepare_native_mod(PrepareNativeModOptions {
        description: options.description,
        declaration: options.declaration,
        declaration_digest: options.declaration_digest,
        program: resource.bytes,
        artifact: resource.reference,
        read_script: Some(read_script),
        localize,
    })
}

struct MountReader {
    mounts: Rc<MountedContent>,
}

impl qa_client::text::resources::LocalizationReader for MountReader {
    fn read(&mut self, path: &str) -> Option<Vec<u8>> {
        self.mounts
            .open(path, |_| true)
            .ok()
            .flatten()
            .map(|resource| resource.bytes)
    }
}

/// Prepare a native mod.
pub fn prepare_native_mod(options: PrepareNativeModOptions) -> Result<NativePreparedMod, NativeModError> {
    let program_digest = format!("sha256:{}", sha256_hex(&options.program));
    let abi = parse_pe(&options.program).map_err(|_| artifact_mismatch())?.abi;
    let classic = match options.declaration.target {
        NativeModTarget::ClassicWindowsI386 => true,
        NativeModTarget::RereleaseWindowsX86_64 => false,
    };
    let abi_matches = matches!(
        (classic, abi),
        (true, qa_guest::core::contracts::NativeAbi::WindowsI386)
            | (false, qa_guest::core::contracts::NativeAbi::WindowsX86_64)
    );
    if options.artifact.digest != options.declaration.program.digest
        || options.artifact.requested_path != options.declaration.program.path
        || program_digest != options.declaration.program.digest.as_str()
        || !abi_matches
    {
        return Err(artifact_mismatch());
    }
    let instance = qa_content::contract::mod_instance_provider(&options.description.selection).map_err(mapped)?;
    let source = ProviderReference {
        provider: instance.clone(),
        content: options.description.source.content.clone(),
    };
    let module = ModuleIdentity {
        id: instance.clone(),
        artifact_path: options.declaration.program.path.clone(),
        digest: options.declaration.program.digest.clone(),
        revision: options.declaration.program.digest.as_str().to_string(),
    };
    let (api, profile) = if classic {
        (
            GameApi::Q2ClassicGame,
            NativeCallAbi::WindowsI386 {
                call: NativeCall::Cdecl,
            },
        )
    } else {
        (GameApi::Q2RereleaseGame, NativeCallAbi::WindowsX8664)
    };
    let execution = ResolvedExecutionModule {
        owner: qa_world::save::shared::ProviderRef {
            provider: format!("{}:{}", source.provider.namespace, source.provider.name),
            content: source.content.as_str().to_string(),
        },
        role: "server-game".to_string(),
        api,
        implementation: ExecutionImplementation::Native {
            artifact: convert_artifact(&options.artifact),
            profile,
        },
    };
    let host = if classic {
        NativeModPrepared::Classic(PreparedClassicGuest {
            edition: PrimaryEdition::Classic,
            primary: None,
            execution,
            bytes: options.program,
        })
    } else {
        NativeModPrepared::Rerelease(PreparedRereleaseGuest {
            edition: PrimaryEdition::Rerelease,
            primary: None,
            execution,
            bytes: options.program,
        })
    };
    Ok(NativePreparedMod {
        data: PreparedMod {
            description: options.description.clone(),
            identity: ModIdentity {
                selection: options.description.selection.clone(),
                source: options.description.source.clone(),
                declaration_digest: options.declaration_digest,
                modules: vec![module],
                providers: vec![ProviderCheckpointHeader {
                    provider: instance,
                    schema: "native:mod".to_string(),
                    version: 1.0,
                }],
            },
            presentation: None,
            module_checkpoint: Some(ModModuleCheckpoint),
            travel: None,
            client_presentation: options
                .declaration
                .client_presentation
                .as_ref()
                .map(|_| ModClientPresentationAdmission),
        },
        host,
        declaration: options.declaration,
        source,
        localize: options.localize,
        read_script: options.read_script,
    })
}

/// Validate a saved native mod state (donor `validateState`). Compat
/// records carry no module binding, so validation covers the record
/// shape plus the API match; provider restore revalidates live state.
pub fn validate_native_mod_state(state: &NativeModSavedState, target: &NativeModTarget) -> Result<(), NativeModError> {
    if state.guests != 0 || state.providers.len() != 1 {
        return Err(NativeModError::invalid("Missing native gameplay mod checkpoint"));
    }
    let record = state
        .providers
        .first()
        .ok_or_else(|| NativeModError::invalid("Missing native gameplay mod checkpoint"))?;
    let expected = if matches!(target, NativeModTarget::ClassicWindowsI386) {
        ApiKind::Classic
    } else {
        ApiKind::Rerelease
    };
    if record.api != expected {
        return Err(NativeModError::invalid("Native mod save ABI differs"));
    }
    Ok(())
}

/// Lower a scalar encoding.
fn convert_scalar(scalar: &NativeModScalar) -> GuestStorage {
    match scalar {
        NativeModScalar::Int8 => GuestStorage::Int8,
        NativeModScalar::Uint8 => GuestStorage::Uint8,
        NativeModScalar::Int16 => GuestStorage::Int16,
        NativeModScalar::Uint16 => GuestStorage::Uint16,
        NativeModScalar::Int32 => GuestStorage::Int32,
        NativeModScalar::Uint32 => GuestStorage::Uint32,
        NativeModScalar::Int64 => GuestStorage::Int64,
        NativeModScalar::Uint64 => GuestStorage::Uint64,
        NativeModScalar::Float32 => GuestStorage::Float32,
        NativeModScalar::Float64 => GuestStorage::Float64,
    }
}

/// Lower an image-relative address.
fn convert_address(address: &NativeModAddress) -> Result<ModAddress, NativeModError> {
    Ok(ModAddress {
        rva: address.rva,
        indirections: address
            .indirections
            .iter()
            .map(|step| {
                i64::try_from(*step).map_err(|_| NativeModError::invalid("Native address indirection exceeds i64"))
            })
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn actor_input_name(input: &qa_content::contract::ModActorInput) -> &'static str {
    match input {
        qa_content::contract::ModActorInput::Slf => "self",
        qa_content::contract::ModActorInput::Other => "other",
        qa_content::contract::ModActorInput::Activator => "activator",
        qa_content::contract::ModActorInput::Attacker => "attacker",
        qa_content::contract::ModActorInput::Inflictor => "inflictor",
    }
}

fn client_input_name(input: &qa_content::contract::ModClientInput) -> &'static str {
    match input {
        qa_content::contract::ModClientInput::ViewAngles => "view-angles",
        qa_content::contract::ModClientInput::Attack => "attack",
        qa_content::contract::ModClientInput::Jump => "jump",
        qa_content::contract::ModClientInput::Impulse => "impulse",
        qa_content::contract::ModClientInput::ForwardMove => "forward-move",
        qa_content::contract::ModClientInput::SideMove => "side-move",
        qa_content::contract::ModClientInput::UpMove => "up-move",
    }
}

/// Donor `ModCallbackInput` strings (`src/contracts/mod-callbacks.ts`).
fn callback_input_name(input: &qa_content::contract::ModCallbackInput) -> String {
    match input {
        qa_content::contract::ModCallbackInput::Slf => "self".to_string(),
        qa_content::contract::ModCallbackInput::Other => "other".to_string(),
        qa_content::contract::ModCallbackInput::Activator => "activator".to_string(),
        qa_content::contract::ModCallbackInput::Attacker => "attacker".to_string(),
        qa_content::contract::ModCallbackInput::Inflictor => "inflictor".to_string(),
        qa_content::contract::ModCallbackInput::Client(input) => client_input_name(input).to_string(),
        qa_content::contract::ModCallbackInput::Amount => "amount".to_string(),
        qa_content::contract::ModCallbackInput::DamageFlags => "damage-flags".to_string(),
        qa_content::contract::ModCallbackInput::RegularProtectionScale => "regular-protection-scale".to_string(),
        qa_content::contract::ModCallbackInput::Knockback => "knockback".to_string(),
        qa_content::contract::ModCallbackInput::Point => "point".to_string(),
        qa_content::contract::ModCallbackInput::Direction => "direction".to_string(),
        qa_content::contract::ModCallbackInput::Normal => "normal".to_string(),
        qa_content::contract::ModCallbackInput::Item => "item".to_string(),
        qa_content::contract::ModCallbackInput::Time => "time".to_string(),
        qa_content::contract::ModCallbackInput::Elapsed => "elapsed".to_string(),
        qa_content::contract::ModCallbackInput::Result => "result".to_string(),
        qa_content::contract::ModCallbackInput::PickupCount => "pickup-count".to_string(),
        qa_content::contract::ModCallbackInput::PickupHasCount => "pickup-has-count".to_string(),
        qa_content::contract::ModCallbackInput::PickupDropped => "pickup-dropped".to_string(),
    }
}

/// Lower a content value to the provider value model. Constants become
/// synthesized `const:{n}` inputs the invoker prefills, since the
/// provider model only names runtime inputs.
fn convert_value(
    value: &NativeModValue,
    consts: &mut Vec<(String, RuntimeValue)>,
) -> Result<ModValueKind, NativeModError> {
    match value {
        NativeModValue::Value { kind, value } => {
            let input = match value {
                qa_content::contract::ModCallbackValue::Input(input) => callback_input_name(input),
                qa_content::contract::ModCallbackValue::Float(constant) => {
                    let name = format!("const:{}", consts.len());
                    consts.push((name.clone(), RuntimeValue::Float(*constant)));
                    name
                }
                qa_content::contract::ModCallbackValue::Str(constant) => {
                    let name = format!("const:{}", consts.len());
                    consts.push((name.clone(), RuntimeValue::Text(constant.0.clone())));
                    name
                }
                qa_content::contract::ModCallbackValue::Vector(constant) => {
                    let name = format!("const:{}", consts.len());
                    consts.push((name.clone(), RuntimeValue::Vector(*constant)));
                    name
                }
            };
            Ok(match kind {
                qa_content::contract::NativeModValueKind::Scalar(_)
                | qa_content::contract::NativeModValueKind::Vector => {
                    // The provider lowers scalars to float64 and vectors to
                    // null writes; the kind selects the input reader.
                    if matches!(kind, qa_content::contract::NativeModValueKind::Vector) {
                        ModValueKind::Vector { input }
                    } else {
                        ModValueKind::Float { input }
                    }
                }
                qa_content::contract::NativeModValueKind::Str => ModValueKind::Text { input },
            })
        }
        NativeModValue::Actor { record, input } => Ok(ModValueKind::Actor {
            input: actor_input_name(input).to_string(),
            record: record.clone(),
        }),
        NativeModValue::Client { input } => Ok(ModValueKind::Client {
            input: actor_input_name(input).to_string(),
        }),
        NativeModValue::Userinfo { input } => Ok(ModValueKind::Userinfo {
            input: actor_input_name(input).to_string(),
        }),
        NativeModValue::UserCommand => Ok(ModValueKind::UserCommand),
        NativeModValue::Time {
            input,
            units: _,
            encoding,
        } => Ok(ModValueKind::Time {
            // Declared units pass through raw; the guest contract reads
            // the declared unit.
            input: match input {
                qa_content::contract::ModTimeInput::Time => "time".to_string(),
                qa_content::contract::ModTimeInput::Elapsed => "elapsed".to_string(),
            },
            encoding: convert_scalar(encoding),
        }),
        NativeModValue::Address(address) => address
            .as_ref()
            .map(|address| convert_address(address).map(ModValueKind::Address))
            .transpose()?
            .ok_or_else(|| NativeModError::invalid("Native null address values need a declared address")),
    }
}

/// Lower a content call to the provider call model.
fn convert_call(call: &NativeModSourceCall) -> Result<(ModCallDecl, Vec<(String, RuntimeValue)>), NativeModError> {
    let mut consts = Vec::new();
    let entry = match &call.entry {
        NativeModEntry::Export { name } => ModEntryRef::Export(name.clone()),
        NativeModEntry::Rva { rva } => ModEntryRef::Rva(*rva),
        NativeModEntry::GameExport { name } => {
            return Err(NativeModError::invalid(format!(
                "Native game-export call {name} needs the export table"
            )));
        }
    };
    let id = match &call.entry {
        NativeModEntry::Export { name } => format!("export:{name}"),
        NativeModEntry::Rva { rva } => format!("rva:{rva:#x}"),
        NativeModEntry::GameExport { name } => format!("game-export:{name}"),
    };
    let mut arguments = Vec::with_capacity(call.arguments.len());
    for argument in &call.arguments {
        arguments.push(convert_value(argument, &mut consts)?);
    }
    let mut globals = Vec::with_capacity(call.globals.len());
    for global in &call.globals {
        globals.push(convert_global(global, &mut consts)?);
    }
    Ok((
        ModCallDecl {
            id,
            entry,
            accepts: CallAccepts::Always,
            returns: match &call.returns {
                NativeModReturn::Void => CallReturns::Void,
                // The provider reports integer decisions; scalar returns
                // collapse to the decision word.
                NativeModReturn::Scalar(_) => CallReturns::Int32,
            },
            skips: call
                .skips
                .iter()
                .map(|skip| qa_compat::q2::native_mod_provider::ModSkip {
                    entry: skip.entry,
                    join: skip.join,
                })
                .collect(),
            arguments,
            globals,
        },
        consts,
    ))
}

/// Lower a content global. Globals roundtrip through one scalar word,
/// so only scalar values lower.
fn convert_global(
    global: &NativeModGlobal,
    consts: &mut Vec<(String, RuntimeValue)>,
) -> Result<ModGlobal, NativeModError> {
    let NativeModValue::Value { kind, .. } = &global.value else {
        return Err(NativeModError::invalid("Native global values must be scalar"));
    };
    let qa_content::contract::NativeModValueKind::Scalar(scalar) = kind else {
        return Err(NativeModError::invalid("Native global values must be scalar"));
    };
    let encoding = convert_scalar(scalar);
    Ok(ModGlobal {
        address: convert_address(&global.address)?,
        encoding,
        value: convert_value(&global.value, consts)?,
    })
}

use qa_guest::core::memory::SparseGuestMemory;

use super::classic_guest_services::ClassicGuestMessage;
use super::native_mod_host::{create_native_mod_host, ModHostServices, NativeModPresentationSlot as SlotInputs};
use super::native_mod_presentation::{
    ModClientPresentationFrame, NativeModAppearance, NativeModEntityState, NativeQ2PresentationClock,
};

/// Initialize options (donor `initialize` context).
pub struct NativeModInitContext<S> {
    /// Mod services.
    pub services: ModHostServices,
    /// Host context.
    pub native: NativeModHostContext,
    /// Provider services (session lane supplies live services
    /// post-merge; synthetic services work standalone).
    pub provider_services: S,
    /// Resource tracker.
    pub resources: ModResourceServices,
    /// Declaration converter seam.
    pub convert_declaration: ConvertDeclarationFn,
    /// Command-port binder.
    pub bind_commands: Option<BindCommandsFn>,
    /// Cvar capture seam.
    pub capture_cvars: NativeModCvarCaptureFn,
    /// Cvar restore seam.
    pub restore_cvars: NativeModCvarRestoreFn,
    /// Service translation seam.
    pub translate: super::native_mod_presentation::TranslateQ2ServiceRecordsFn,
    /// Wire fog seam.
    pub fog_from_wire: super::native_mod_presentation::Q2FogFromWireFn,
    /// Frame pump.
    pub next_frame: Box<dyn FnMut()>,
    /// Whether a save follows.
    pub restoring: bool,
    /// Session-current guard seam.
    pub assert_current: AssertCurrentFn,
}

/// Raw host link for the provider, which owns its host handle by
/// value. The instance boxes the host so the address is stable; every
/// method below dereferences call-scoped borrows only.
///
/// SOUNDNESS: single-threaded (`*mut` is `!Send`/`!Sync`, so the link
/// never crosses threads). The instance never holds a host borrow
/// across a provider call, the provider never calls back into the
/// instance, and every dereference below is confined to one method
/// body with no live alias. Session-held handles (invokers, client
/// sources) check the shared closed flag before dereferencing, and
/// `Drop` sets it, so use-after-close cannot dereference.
#[derive(Clone, Copy)]
pub struct HostLink {
    host: *mut NativeModHost,
}

impl HostLink {
    /// Exclusive host access, tied to the handle borrow.
    fn host_mut(&mut self) -> &mut NativeModHost {
        // SAFETY: see the type-level argument on `HostLink`.
        unsafe { &mut *self.host }
    }

    /// Shared host access, tied to the handle borrow.
    fn host_ref(&self) -> &NativeModHost {
        // SAFETY: see the type-level argument on `HostLink`.
        unsafe { &*self.host }
    }
}

impl ProviderHost for HostLink {
    fn memory(&mut self) -> &mut SparseGuestMemory {
        self.host_mut().memory()
    }

    fn image_base(&self) -> GuestAddress {
        self.host_ref().image_base()
    }

    fn entry(&self, name: &str) -> Option<GuestAddress> {
        self.host_ref().entry(name).ok()
    }

    fn entities(&self) -> qa_compat::q2::native_mod_provider::EntityTable {
        // SAFETY: statement-scoped; see the type-level argument on
        // `HostLink`. The table read fails only before the first
        // spawn; the provider always runs post-spawn.
        unsafe { &mut *self.host }
            .entities()
            .unwrap_or(qa_compat::q2::native_mod_provider::EntityTable {
                base: GuestAddress { space: 0, offset: 0 },
                stride: 0,
                count: 0,
                capacity: 0,
            })
    }

    fn weapon_model(&self, slot: usize) -> u32 {
        self.host_ref().weapon_model(slot)
    }

    fn invoke_entry(&mut self, address: GuestAddress, values: &[GuestCallValue]) -> GuestCallResult {
        self.host_mut().invoke_entry(address, values)
    }
}

/// Provider-backed slot projection (donor `NativeModProvider` as
/// `NativeModProjection`). Core actors are minted by a lane-local
/// owner; the session lane substitutes shared authorities post-merge.
pub struct ProviderProjection<S: ProviderServices> {
    provider: Rc<RefCell<NativeModProvider<HostLink, S>>>,
    host: HostLink,
    owner: IdentityOwner,
    actors: Rc<RefCell<HashMap<u32, ActorId>>>,
    natives: Rc<RefCell<HashMap<ActorId, NativeActorId>>>,
    instance: ProviderId,
    entity_record: Option<String>,
    is_live: Rc<dyn Fn(&ActorId) -> bool>,
    players: Option<Rc<dyn Fn() -> Vec<ActorId>>>,
}

impl<S> ProviderProjection<S>
where
    S: ProviderServices,
{
    fn project_owned(&self, slot: u32) -> Option<OwnedActor> {
        if slot == 0 {
            // The engine world actor lives behind the foundation host,
            // which the services own without an accessor; slot zero
            // stays unprojected.
            return None;
        }
        if let Some(actor) = self.actors.borrow().get(&slot).cloned() {
            return self.owner.owned_actor(&actor, self.instance.clone()).ok();
        }
        let native = self.provider.borrow_mut().project(slot as usize).ok()?;
        let actor = self.owner.actor(slot, native.id.generation);
        self.actors.borrow_mut().insert(slot, actor.clone());
        self.natives.borrow_mut().insert(actor.clone(), native.id);
        self.owner.owned_actor(&actor, self.instance.clone()).ok()
    }
}

impl<S> NativeModProjection for ProviderProjection<S>
where
    S: ProviderServices,
{
    fn project(&self, record: &qa_compat::q2::classic::records::RawEntityView) -> Option<OwnedActor> {
        self.project_owned(record.slot)
    }

    fn actor_at(&self, slot: u32) -> Option<ActorId> {
        if slot == 0 {
            return None;
        }
        self.actors
            .borrow()
            .get(&slot)
            .cloned()
            .filter(|actor| (self.is_live)(actor))
    }

    fn slot_of(&self, actor: &ActorId) -> Option<u32> {
        let native = self.natives.borrow().get(actor).copied()?;
        self.provider
            .borrow()
            .slot_of(native)
            .and_then(|slot| u32::try_from(slot).ok())
    }

    fn accepts_client(&self, slot: u32) -> bool {
        let Some(actor) = self.actor_at(slot) else {
            return false;
        };
        let admitted = self
            .natives
            .borrow()
            .get(&actor)
            .copied()
            .is_some_and(|native| self.provider.borrow().client_admitted(native));
        admitted || self.players.as_ref().is_some_and(|players| players().contains(&actor))
    }

    fn address(&self, actor: &ActorId) -> Option<GuestAddress> {
        let slot = self.slot_of(actor)?;
        match self.entity_record.as_ref() {
            Some(record) => {
                let native = self.natives.borrow().get(actor).copied()?;
                self.provider.borrow_mut().pointer(native, record)
            }
            None => {
                // SAFETY: call-scoped; see `HostLink`.
                unsafe { &mut *self.host.host }
                    .entity(slot)
                    .ok()
                    .map(|record| record.address)
            }
        }
    }

    fn import_boundary(
        &self,
        _name: &str,
        _values: &[GuestCallValue],
        invoke: &dyn Fn() -> GuestCallResult,
    ) -> GuestCallResult {
        // The compat provider models no import boundary; calls run
        // through unwrapped.
        invoke()
    }
}

/// Snapshot presentation source: owned inputs built from the host in
/// one borrow, served to the presentation in the next.
pub struct HostPresentationSource {
    edition: ModEdition,
    configstrings: HashMap<i32, String>,
    messages: Vec<ClassicGuestMessage>,
    slots: HashMap<u32, SlotInputs>,
    clock: NativeQ2PresentationClock,
}

impl super::native_mod_presentation::NativeModPresentationSource for HostPresentationSource {
    fn edition(&self) -> ModEdition {
        self.edition
    }

    fn configstrings(&mut self) -> HashMap<i32, String> {
        self.configstrings.clone()
    }

    fn drain_messages(&mut self) -> Vec<ClassicGuestMessage> {
        std::mem::take(&mut self.messages)
    }

    fn appearance(
        &mut self,
        slot: u32,
    ) -> Result<NativeModAppearance, super::native_mod_presentation::NativeModPresentationError> {
        self.slots
            .get(&slot)
            .map(|slot| slot.appearance.clone())
            .ok_or_else(|| {
                super::native_mod_presentation::NativeModPresentationError::invalid(
                    "Native mod has no presentation slot",
                )
            })
    }

    fn signature(&mut self, slot: u32) -> Result<String, super::native_mod_presentation::NativeModPresentationError> {
        self.slots.get(&slot).map(|slot| slot.signature.clone()).ok_or_else(|| {
            super::native_mod_presentation::NativeModPresentationError::invalid("Native mod has no presentation slot")
        })
    }

    fn player_state(
        &mut self,
        slot: u32,
    ) -> Result<qa_net::q2_adapters::Q2Player, super::native_mod_presentation::NativeModPresentationError> {
        self.slots.get(&slot).map(|slot| slot.player.clone()).ok_or_else(|| {
            super::native_mod_presentation::NativeModPresentationError::invalid("Native mod has no presentation slot")
        })
    }

    fn clock(&mut self) -> NativeQ2PresentationClock {
        self.clock
    }

    fn entity_state(
        &mut self,
        slot: u32,
    ) -> Result<NativeModEntityState, super::native_mod_presentation::NativeModPresentationError> {
        self.slots.get(&slot).map(|slot| slot.state.clone()).ok_or_else(|| {
            super::native_mod_presentation::NativeModPresentationError::invalid("Native mod has no presentation slot")
        })
    }
}

/// Bundled instance checkpoint: provider state plus the host source
/// save plus the presentation snapshot. The session lane settles the
/// final record shape post-merge.
pub struct NativeModCheckpoint {
    /// Provider checkpoint.
    pub provider: qa_compat::q2::native_mod_provider::ProviderCheckpoint,
    /// Host source save.
    pub source: super::native_mod_host::NativeModSourceSave,
    /// Presentation snapshot.
    pub presentation: NativeModPresentationCheckpoint,
}

/// Live native mod instance (donor `initialize` result).
pub struct NativeModInstance<S: ProviderServices> {
    host: Box<NativeModHost>,
    provider: Rc<RefCell<NativeModProvider<HostLink, S>>>,
    actors: Rc<RefCell<HashMap<u32, ActorId>>>,
    natives: Rc<RefCell<HashMap<ActorId, NativeActorId>>>,
    declaration: NativeModDeclaration,
    assert_current: AssertCurrentFn,
    closed: Rc<Cell<bool>>,
    _guard: CloseGuard,
}

impl<S> NativeModInstance<S>
where
    S: ProviderServices + 'static,
{
    fn link(&mut self) -> HostLink {
        HostLink {
            host: &mut *self.host as *mut NativeModHost,
        }
    }

    fn check_current(&self) -> Result<(), NativeModError> {
        if self.closed.get() {
            return Err(NativeModError::invalid("Native mod is closed"));
        }
        (self.assert_current)();
        Ok(())
    }

    /// Admit a client slot (donor clients flow): ensure the slot is
    /// projected through the bound projection, then admit the native
    /// actor.
    pub fn admit_client(&mut self, slot: u32) -> Result<bool, NativeModError> {
        self.check_current()?;
        if slot == 0 {
            return Ok(false);
        }
        let record = self
            .host
            .entity(slot)
            .map_err(|error| NativeModError::invalid(format!("Native client slot is not bound: {error}")))?;
        // Clone the projection first so no host borrow spans the
        // projection call below (the projection dereferences the host
        // through its own link).
        let projection = Rc::clone(self.host.projection());
        let owned = projection
            .project(&record)
            .ok_or_else(|| NativeModError::invalid("Native client slot is not projected"))?;
        let native = self
            .natives
            .borrow()
            .get(owned.id())
            .cloned()
            .ok_or_else(|| NativeModError::invalid("Native client slot is not projected"))?;
        self.provider.borrow_mut().admit_client(native).map_err(provider_mapped)
    }

    /// Client presentation source (donor `clientPresentation`).
    pub fn client_presentation(&mut self) -> NativeModClientSource<S> {
        NativeModClientSource {
            host: self.link(),
            provider: Rc::clone(&self.provider),
            natives: Rc::clone(&self.natives),
            assert_current: Rc::clone(&self.assert_current),
            closed: Rc::clone(&self.closed),
        }
    }

    /// Advance one frame (donor `advance`).
    pub fn advance(&mut self, seconds: f64, frame: u32) -> Result<(), NativeModError> {
        self.check_current()?;
        self.host.synchronize_frame(seconds, frame);
        self.provider.borrow_mut().release_pending().map_err(provider_mapped)?;
        self.provider
            .borrow_mut()
            .advance_owned(seconds)
            .map_err(provider_mapped)?;
        let mut slots: Vec<u32> = self.actors.borrow().keys().copied().collect();
        slots.sort_unstable();
        for slot in slots {
            self.host
                .clear_entity_event(slot)
                .map_err(|error| NativeModError::invalid(format!("Native mod advance failed: {error}")))?;
        }
        self.host.presentation_mut().begin_frame();
        Ok(())
    }

    fn appearances(&mut self) -> Result<Vec<super::types::SimulationPresentation>, NativeModError> {
        let mut slots: Vec<u32> = self.actors.borrow().keys().copied().collect();
        slots.sort_unstable();
        let mut snapshots = HashMap::new();
        for slot in &slots {
            let inputs = self
                .host
                .presentation_slot(*slot)
                .map_err(|error| NativeModError::invalid(format!("Native mod presentation failed: {error}")))?;
            snapshots.insert(*slot, inputs);
        }
        let drain = self
            .host
            .presentation_drain()
            .map_err(|error| NativeModError::invalid(format!("Native mod presentation failed: {error}")))?;
        let mut source = HostPresentationSource {
            edition: self.host.edition(),
            configstrings: drain.configstrings,
            messages: drain.messages,
            slots: snapshots,
            clock: drain.clock,
        };
        let mut out = Vec::new();
        for slot in slots {
            let Some(actor) = self.actors.borrow().get(&slot).cloned() else {
                continue;
            };
            let mut frame = self
                .host
                .presentation_mut()
                .appearance(&mut source, &actor, slot)
                .map_err(mapped)?;
            out.append(&mut frame);
        }
        Ok(out)
    }

    /// Present every bound-live actor (donor `presentations`).
    pub fn presentations(&mut self) -> Result<Vec<super::types::SimulationPresentation>, NativeModError> {
        self.check_current()?;
        self.appearances()
    }

    /// Appearance overrides (donor `appearanceOverrides`). The compat
    /// provider tracks no appearance-dirty set, so every bound-live
    /// actor publishes.
    pub fn appearance_overrides(&mut self) -> Result<Vec<super::types::SimulationPresentation>, NativeModError> {
        self.check_current()?;
        self.appearances()
    }

    /// Invoke a content call (donor `provider.invoke`, without the
    /// client-rejects gate, which has no compat counterpart).
    pub fn invoke(
        &mut self,
        call: &NativeModSourceCall,
        inputs: HashMap<String, RuntimeValue>,
    ) -> Result<Option<i32>, NativeModError> {
        self.check_current()?;
        let (lowered, consts) = convert_call(call)?;
        let mut merged = inputs;
        for (name, value) in consts {
            merged.insert(name, value);
        }
        self.provider
            .borrow_mut()
            .execute(&lowered, &merged)
            .map_err(provider_mapped)
    }

    /// Register callback invokers with the session table.
    pub fn register(&mut self, sink: &mut dyn ModCallbackSink) -> Result<(), NativeModError> {
        self.check_current()?;
        let callbacks = self.declaration.callbacks.clone();
        for callback in &callbacks {
            let (lowered, consts) = convert_call(&callback.call)?;
            sink.add(callback, self.invoker(lowered, consts));
        }
        Ok(())
    }

    fn invoker(&mut self, call: ModCallDecl, consts: Vec<(String, RuntimeValue)>) -> ModCallbackInvoker {
        let host = self.link();
        let provider = Rc::clone(&self.provider);
        let assert = Rc::clone(&self.assert_current);
        let closed = Rc::clone(&self.closed);
        Rc::new(move |inputs| {
            if closed.get() {
                return Err(NativeModError::invalid("Native mod is closed"));
            }
            assert();
            let mut merged = inputs;
            for (name, value) in &consts {
                merged.insert(name.clone(), value.clone());
            }
            // SAFETY: the closed flag above guarantees the instance is
            // alive; the time read is call-scoped. See `HostLink`.
            let now = unsafe { &mut *host.host }.source_time();
            merged
                .entry("time".to_string())
                .or_insert_with(|| RuntimeValue::Float(now));
            provider.borrow_mut().execute(&call, &merged).map_err(provider_mapped)
        })
    }

    /// Capture the bundled checkpoint.
    pub fn checkpoint(&mut self) -> Result<NativeModCheckpoint, NativeModError> {
        self.check_current()?;
        let provider = self.provider.borrow_mut().checkpoint().map_err(provider_mapped)?;
        let source = self
            .host
            .checkpoint()
            .map_err(|error| NativeModError::invalid(format!("Native mod checkpoint failed: {error}")))?;
        let presentation = self.host.presentation().checkpoint();
        Ok(NativeModCheckpoint {
            provider,
            source,
            presentation,
        })
    }

    /// Restore the bundled checkpoint.
    pub fn restore(&mut self, checkpoint: &NativeModCheckpoint) -> Result<(), NativeModError> {
        self.check_current()?;
        validate_native_mod_state(
            &NativeModSavedState {
                guests: 0,
                providers: vec![checkpoint.provider.clone()],
            },
            &self.declaration.target,
        )?;
        self.host
            .restore(&checkpoint.source)
            .map_err(|error| NativeModError::invalid(format!("Native mod restore failed: {error}")))?;
        self.provider
            .borrow_mut()
            .restore(&checkpoint.provider)
            .map_err(provider_mapped)?;
        self.host
            .presentation_mut()
            .restore(&checkpoint.presentation)
            .map_err(mapped)?;
        Ok(())
    }

    /// Close the instance.
    pub fn close(&mut self) -> Result<(), NativeModError> {
        self.check_current()?;
        self.provider.borrow_mut().close().map_err(provider_mapped)?;
        self.host
            .close()
            .map_err(|error| NativeModError::invalid(format!("Native mod close failed: {error}")))?;
        self.closed.set(true);
        Ok(())
    }
}

/// Sets the shared closed flag when the instance drops, so
/// session-held invokers and client sources stop dereferencing.
struct CloseGuard {
    closed: Rc<Cell<bool>>,
}

impl Drop for CloseGuard {
    fn drop(&mut self) {
        self.closed.set(true);
    }
}

/// Client presentation source (donor `clientPresentation` result).
pub struct NativeModClientSource<S: ProviderServices> {
    host: HostLink,
    provider: Rc<RefCell<NativeModProvider<HostLink, S>>>,
    natives: Rc<RefCell<HashMap<ActorId, NativeActorId>>>,
    assert_current: AssertCurrentFn,
    closed: Rc<Cell<bool>>,
}

impl<S> NativeModClientSource<S>
where
    S: ProviderServices,
{
    /// Presentation generation (zero once closed).
    #[must_use]
    pub fn generation(&self) -> u32 {
        if self.closed.get() {
            return 0;
        }
        (self.assert_current)();
        // SAFETY: the closed flag above guarantees the instance is
        // alive; the read is call-scoped. See `HostLink`.
        unsafe { &*self.host.host }.presentation().generation()
    }

    /// Client frame for an admitted actor.
    pub fn frame(&self, actor: &ActorId) -> Option<ModClientPresentationFrame> {
        if self.closed.get() {
            return None;
        }
        (self.assert_current)();
        let native = self.natives.borrow().get(actor).copied()?;
        if !self.provider.borrow().client_admitted(native) {
            return None;
        }
        // SAFETY: the closed flag above guarantees the instance is
        // alive; the read is call-scoped. See `HostLink`.
        unsafe { &*self.host.host }.presentation().client_frame(actor)
    }
}

/// Initialize a prepared native mod (donor `initialize`).
pub fn initialize_native_mod<S>(
    prepared: NativePreparedMod,
    context: NativeModInitContext<S>,
) -> Result<NativeModInstance<S>, NativeModError>
where
    S: ProviderServices + 'static,
{
    (context.resources.own)(format!(
        "{}:{}",
        prepared.source.provider.namespace, prepared.source.provider.name
    ));
    let provider_declaration = (context.convert_declaration)(&prepared.declaration)?;
    let entity_record = provider_declaration.entity_record.clone();
    let map = context.native.map_path.clone();
    let is_live = Rc::clone(&context.services.actors.is_live);
    let players = context
        .services
        .engine
        .as_ref()
        .and_then(|engine| engine.presentation_players.clone());
    let instance_id = prepared.source.provider.clone();
    let shared = SharedProjection::new();
    let mut host = create_native_mod_host(NativeModHostOptions {
        prepared: prepared.host,
        declaration: prepared.declaration.clone(),
        source: prepared.source.clone(),
        context: context.native,
        services: context.services,
        projection: shared.handle(),
        bind_commands: context.bind_commands,
        localize: prepared.localize,
        next_frame: context.next_frame,
        capture_cvars: context.capture_cvars,
        restore_cvars: context.restore_cvars,
        translate: context.translate,
        fog_from_wire: context.fog_from_wire,
    })
    .map_err(|error| NativeModError::invalid(format!("Native mod host failed: {error}")))?;
    host.initialize(context.restoring)
        .map_err(|error| NativeModError::invalid(format!("Native mod host failed: {error}")))?;
    let mut host = Box::new(host);
    let link = HostLink {
        host: &mut *host as *mut NativeModHost,
    };
    let instance_name = format!("{}:{}", instance_id.namespace, instance_id.name);
    let provider = NativeModProvider::new(
        provider_declaration.clone(),
        link,
        context.provider_services,
        &instance_name,
        &map,
    )
    .map_err(provider_mapped)?;
    let provider = Rc::new(RefCell::new(provider));
    if !context.restoring {
        provider.borrow_mut().begin_lifecycle().map_err(provider_mapped)?;
    }
    preflight_calls(&provider, link, &provider_declaration)?;
    let actors: Rc<RefCell<HashMap<u32, ActorId>>> = Rc::new(RefCell::new(HashMap::new()));
    let natives: Rc<RefCell<HashMap<ActorId, NativeActorId>>> = Rc::new(RefCell::new(HashMap::new()));
    let projection = ProviderProjection {
        provider: Rc::clone(&provider),
        host: link,
        owner: IdentityOwner::create(&instance_name).map_err(mapped)?,
        actors: Rc::clone(&actors),
        natives: Rc::clone(&natives),
        instance: instance_id,
        entity_record,
        is_live,
        players,
    };
    shared.set(Rc::new(projection));
    let closed = Rc::new(Cell::new(false));
    Ok(NativeModInstance {
        host,
        provider,
        actors,
        natives,
        declaration: prepared.declaration,
        assert_current: context.assert_current,
        closed: Rc::clone(&closed),
        _guard: CloseGuard { closed },
    })
}

/// Resolve every provider call entry and check execute access (donor
/// entry preflight in `initialize`).
fn preflight_calls<S>(
    provider: &Rc<RefCell<NativeModProvider<HostLink, S>>>,
    link: HostLink,
    declaration: &ProviderDeclaration,
) -> Result<(), NativeModError>
where
    S: ProviderServices + 'static,
{
    let mut guard = provider.borrow_mut();
    let mut calls: Vec<&ModCallDecl> = Vec::new();
    calls.extend(declaration.initialize.iter());
    calls.extend(declaration.project.iter());
    calls.extend(declaration.release.iter());
    calls.extend(declaration.callbacks.iter().map(|callback| &callback.call));
    if let Some(clients) = declaration.clients.as_ref() {
        calls.extend(clients.admit.iter());
        calls.extend(clients.userinfo.iter());
        calls.extend(clients.disconnect.iter());
        calls.extend(clients.command.iter());
        calls.extend(clients.frame.iter());
        calls.extend(clients.end_frame.iter());
        for binding in &clients.input {
            calls.extend(binding.calls.iter());
        }
    }
    for call in calls {
        let target = guard.resolve_entry(&call.entry).map_err(provider_mapped)?;
        // SAFETY: call-scoped; see `HostLink`.
        unsafe { &mut *link.host }
            .memory()
            .check(target, 1, qa_guest::core::contracts::GuestAccess::Execute)
            .map_err(|error| NativeModError::invalid(format!("Native mod entry preflight failed: {error}")))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use qa_content::contract::{
        ArchiveFormat, ContentId, LooseMount, ModAvailability, ModCallbackInput, ModCallbackString, ModCallbackValue,
        ModClientInput, ModProgram, ModPurpose, ModSelection, MountId, MountIdentity, NativeModValueKind, ResourceId,
    };
    use qa_core::math::Vec3;

    use super::super::q2_native_world::fixtures::minimal_pe;
    use super::*;

    fn provider_id() -> ProviderId {
        ProviderId {
            namespace: "test".to_string(),
            name: "mod".to_string(),
        }
    }

    fn selection() -> ModSelection {
        ModSelection {
            product: "q2".to_string(),
            id: "testmod".to_string(),
        }
    }

    fn description() -> ModDescription {
        ModDescription {
            selection: selection(),
            source: ProviderReference {
                provider: provider_id(),
                content: ContentId("q2:test:baseq2:1".to_string()),
            },
            title: "Test".to_string(),
            source_title: "Test".to_string(),
            purpose: ModPurpose::GameType,
            requires: Vec::new(),
            conflicts: Vec::new(),
            availability: ModAvailability::Available,
        }
    }

    fn declaration(program_digest: ContentDigest) -> NativeModDeclaration {
        NativeModDeclaration {
            objectives: Vec::new(),
            client_presentation: None,
            version: 1,
            program: ModProgram {
                path: "game.dll".to_string(),
                digest: program_digest,
            },
            target: NativeModTarget::ClassicWindowsI386,
            source_actors: None,
            protection: Vec::new(),
            pickups: Vec::new(),
            items: None,
            clients: None,
            cvars: Vec::new(),
            spawn_entities: None,
            actor_records: Vec::new(),
            entity_record: None,
            initialize: Vec::new(),
            project: Vec::new(),
            release: Vec::new(),
            callbacks: Vec::new(),
        }
    }

    fn artifact(digest: ContentDigest) -> ResolvedResourceReference {
        ResolvedResourceReference {
            id: ResourceId("resource:game".to_string()),
            requested_path: "game.dll".to_string(),
            provenance: qa_content::contract::ResourceProvenance::Loose {
                mount: LooseMount {
                    identity: MountIdentity {
                        id: MountId("mount:test:m".to_string()),
                        content: ContentId("q2:test:baseq2:1".to_string()),
                        generation: 3,
                    },
                    root_path: "/tmp".to_string(),
                },
                member_path: "game.dll".to_string(),
            },
            digest,
            byte_length: 1024,
            resolution: qa_content::contract::ResourceResolution::DefaultOrder {
                plan: qa_content::contract::MountPlanId("mount-plan:test:1".to_string()),
                rank: 0,
            },
        }
    }

    fn localize() -> GuestLocalize {
        Rc::new(|key, _| key.to_string())
    }

    #[test]
    fn prepare_accepts_matching_artifact() {
        let program = minimal_pe(0x14c, 0x10b, &["GetGameAPI"]);
        let digest = ContentDigest(format!("sha256:{}", sha256_hex(&program)));
        let prepared = prepare_native_mod(PrepareNativeModOptions {
            description: description(),
            declaration: declaration(digest.clone()),
            declaration_digest: ContentDigest("sha256:decl".to_string()),
            program,
            artifact: artifact(digest.clone()),
            read_script: None,
            localize: localize(),
        })
        .expect("prepare");
        assert!(matches!(prepared.host, NativeModPrepared::Classic(_)));
        assert_eq!(prepared.data.identity.modules.len(), 1);
        assert_eq!(prepared.data.identity.modules[0].digest, digest);
        assert_eq!(prepared.data.identity.providers.len(), 1);
        assert!(prepared.data.module_checkpoint.is_some());
        assert!(prepared.data.client_presentation.is_none());
        assert_eq!(prepared.source.content.as_str(), "q2:test:baseq2:1");
    }

    #[test]
    fn prepare_rejects_mismatches() {
        let program = minimal_pe(0x14c, 0x10b, &["GetGameAPI"]);
        let digest = ContentDigest(format!("sha256:{}", sha256_hex(&program)));
        let options = || PrepareNativeModOptions {
            description: description(),
            declaration: declaration(digest.clone()),
            declaration_digest: ContentDigest("sha256:decl".to_string()),
            program: program.clone(),
            artifact: artifact(digest.clone()),
            read_script: None,
            localize: localize(),
        };
        let mut bad_digest = options();
        bad_digest.artifact.digest = ContentDigest("sha256:other".to_string());
        assert!(prepare_native_mod(bad_digest).is_err());
        let mut bad_path = options();
        bad_path.artifact.requested_path = "other.dll".to_string();
        assert!(prepare_native_mod(bad_path).is_err());
        let mut bad_bytes = options();
        bad_bytes.program[64] ^= 0xff;
        assert!(prepare_native_mod(bad_bytes).is_err());
        let mut bad_abi = options();
        bad_abi.program = minimal_pe(0x8664, 0x20b, &["GetGameAPI"]);
        let changed = ContentDigest(format!("sha256:{}", sha256_hex(&bad_abi.program)));
        bad_abi.declaration.program.digest = changed.clone();
        bad_abi.artifact.digest = changed;
        assert!(prepare_native_mod(bad_abi).is_err());
    }

    #[test]
    fn prepare_selects_rerelease_guest() {
        let program = minimal_pe(0x8664, 0x20b, &["GetGameAPI"]);
        let digest = ContentDigest(format!("sha256:{}", sha256_hex(&program)));
        let mut declaration = declaration(digest.clone());
        declaration.target = NativeModTarget::RereleaseWindowsX86_64;
        let prepared = prepare_native_mod(PrepareNativeModOptions {
            description: description(),
            declaration,
            declaration_digest: ContentDigest("sha256:decl".to_string()),
            program,
            artifact: artifact(digest),
            read_script: None,
            localize: localize(),
        })
        .expect("prepare");
        assert!(matches!(prepared.host, NativeModPrepared::Rerelease(_)));
    }

    fn source_call() -> NativeModSourceCall {
        NativeModSourceCall {
            entry: NativeModEntry::Export {
                name: "ClientThink".to_string(),
            },
            arguments: vec![
                NativeModValue::Value {
                    kind: NativeModValueKind::Scalar(NativeModScalar::Int32),
                    value: ModCallbackValue::Input(ModCallbackInput::Slf),
                },
                NativeModValue::Value {
                    kind: NativeModValueKind::Scalar(NativeModScalar::Float32),
                    value: ModCallbackValue::Float(1.5),
                },
            ],
            globals: Vec::new(),
            returns: NativeModReturn::Void,
            skips: vec![qa_content::contract::NativeModSkip { entry: 8, join: 16 }],
        }
    }

    #[test]
    fn convert_call_lowers_entries_and_consts() {
        let (call, consts) = convert_call(&source_call()).expect("convert");
        assert_eq!(call.id, "export:ClientThink");
        assert!(matches!(call.entry, ModEntryRef::Export(_)));
        assert_eq!(call.arguments.len(), 2);
        assert!(matches!(
            call.arguments[0],
            ModValueKind::Float { ref input } if input == "self"
        ));
        assert!(matches!(call.arguments[1], ModValueKind::Float { .. }));
        assert_eq!(consts.len(), 1);
        assert_eq!(consts[0].0, "const:0");
        assert_eq!(call.skips.len(), 1);
        assert_eq!(call.skips[0].entry, 8);
        assert!(matches!(call.returns, CallReturns::Void));
        let rva = NativeModSourceCall {
            entry: NativeModEntry::Rva { rva: 0x1234 },
            arguments: Vec::new(),
            globals: Vec::new(),
            returns: NativeModReturn::Scalar(NativeModScalar::Int32),
            skips: Vec::new(),
        };
        let (call, _) = convert_call(&rva).expect("rva");
        assert_eq!(call.id, "rva:0x1234");
        assert!(matches!(call.returns, CallReturns::Int32));
    }

    #[test]
    fn convert_call_rejects_game_exports() {
        let call = NativeModSourceCall {
            entry: NativeModEntry::GameExport {
                name: "RunFrame".to_string(),
            },
            arguments: Vec::new(),
            globals: Vec::new(),
            returns: NativeModReturn::Void,
            skips: Vec::new(),
        };
        assert!(convert_call(&call).is_err());
    }

    #[test]
    fn convert_value_covers_shapes() {
        let mut consts = Vec::new();
        let actor = convert_value(
            &NativeModValue::Actor {
                record: "r".to_string(),
                input: qa_content::contract::ModActorInput::Attacker,
            },
            &mut consts,
        )
        .expect("actor");
        assert!(matches!(
            actor,
            ModValueKind::Actor { ref input, .. } if input == "attacker"
        ));
        let client = convert_value(
            &NativeModValue::Client {
                input: qa_content::contract::ModActorInput::Slf,
            },
            &mut consts,
        )
        .expect("client");
        assert!(matches!(client, ModValueKind::Client { .. }));
        let userinfo = convert_value(
            &NativeModValue::Userinfo {
                input: qa_content::contract::ModActorInput::Other,
            },
            &mut consts,
        )
        .expect("userinfo");
        assert!(matches!(userinfo, ModValueKind::Userinfo { .. }));
        assert!(matches!(
            convert_value(&NativeModValue::UserCommand, &mut consts).expect("usercmd"),
            ModValueKind::UserCommand
        ));
        let time = convert_value(
            &NativeModValue::Time {
                input: qa_content::contract::ModTimeInput::Elapsed,
                units: qa_content::contract::ModTimeUnits::Milliseconds,
                encoding: NativeModScalar::Float64,
            },
            &mut consts,
        )
        .expect("time");
        assert!(matches!(
            time,
            ModValueKind::Time { ref input, .. } if input == "elapsed"
        ));
        let text = convert_value(
            &NativeModValue::Value {
                kind: NativeModValueKind::Str,
                value: ModCallbackValue::Str(ModCallbackString("hi".to_string())),
            },
            &mut consts,
        )
        .expect("text");
        assert!(matches!(text, ModValueKind::Text { .. }));
        let vector = convert_value(
            &NativeModValue::Value {
                kind: NativeModValueKind::Vector,
                value: ModCallbackValue::Vector(Vec3 { x: 1.0, y: 0.0, z: 0.0 }),
            },
            &mut consts,
        )
        .expect("vector");
        assert!(matches!(vector, ModValueKind::Vector { .. }));
        let view = convert_value(
            &NativeModValue::Value {
                kind: NativeModValueKind::Vector,
                value: ModCallbackValue::Input(ModCallbackInput::Client(ModClientInput::ViewAngles)),
            },
            &mut consts,
        )
        .expect("view");
        assert!(matches!(
            view,
            ModValueKind::Vector { ref input } if input == "view-angles"
        ));
        assert_eq!(consts.len(), 2);
        assert!(convert_value(&NativeModValue::Address(None), &mut consts).is_err());
        let address = convert_value(
            &NativeModValue::Address(Some(NativeModAddress {
                rva: 64,
                indirections: vec![8],
            })),
            &mut consts,
        )
        .expect("address");
        assert!(matches!(address, ModValueKind::Address(_)));
    }

    #[test]
    fn convert_global_requires_scalar() {
        let mut consts = Vec::new();
        let scalar = convert_global(
            &NativeModGlobal {
                address: NativeModAddress {
                    rva: 64,
                    indirections: Vec::new(),
                },
                value: NativeModValue::Value {
                    kind: NativeModValueKind::Scalar(NativeModScalar::Int32),
                    value: ModCallbackValue::Input(ModCallbackInput::Time),
                },
            },
            &mut consts,
        )
        .expect("scalar");
        assert_eq!(scalar.address.rva, 64);
        let vector = convert_global(
            &NativeModGlobal {
                address: NativeModAddress {
                    rva: 64,
                    indirections: Vec::new(),
                },
                value: NativeModValue::Value {
                    kind: NativeModValueKind::Vector,
                    value: ModCallbackValue::Input(ModCallbackInput::Point),
                },
            },
            &mut consts,
        );
        assert!(vector.is_err());
        assert!(convert_address(&NativeModAddress {
            rva: 1,
            indirections: vec![u64::MAX]
        })
        .is_err());
    }

    #[test]
    fn callback_input_names_match_donor() {
        assert_eq!(callback_input_name(&ModCallbackInput::DamageFlags), "damage-flags");
        assert_eq!(
            callback_input_name(&ModCallbackInput::RegularProtectionScale),
            "regular-protection-scale"
        );
        assert_eq!(
            callback_input_name(&ModCallbackInput::PickupHasCount),
            "pickup-has-count"
        );
        assert_eq!(callback_input_name(&ModCallbackInput::Result), "result");
    }

    #[test]
    fn convert_artifact_maps_mounts() {
        let digest = ContentDigest("sha256:abc".to_string());
        let record = convert_artifact(&artifact(digest));
        assert_eq!(record.id, "resource:game");
        assert_eq!(record.digest, "sha256:abc");
        assert!(matches!(
            record.provenance,
            crate::persistence::recipe::ResourceProvenance::Loose { .. }
        ));
        assert!(matches!(
            record.resolution,
            crate::persistence::recipe::ResourceResolution::DefaultOrder { .. }
        ));
        let _ = ArchiveFormat::Pak;
    }

    fn provider_checkpoint(api: ApiKind) -> qa_compat::q2::native_mod_provider::ProviderCheckpoint {
        qa_compat::q2::native_mod_provider::ProviderCheckpoint {
            version: 1,
            map: "q2dm1".to_string(),
            api,
            actors: Vec::new(),
            clients: Vec::new(),
            owned: None,
            shared: HashMap::new(),
        }
    }

    #[test]
    fn validate_state_checks_shape_and_api() {
        let state = NativeModSavedState {
            guests: 0,
            providers: vec![provider_checkpoint(ApiKind::Classic)],
        };
        assert!(validate_native_mod_state(&state, &NativeModTarget::ClassicWindowsI386).is_ok());
        assert!(validate_native_mod_state(&state, &NativeModTarget::RereleaseWindowsX86_64).is_err());
        let guests = NativeModSavedState {
            guests: 1,
            providers: vec![provider_checkpoint(ApiKind::Classic)],
        };
        assert!(validate_native_mod_state(&guests, &NativeModTarget::ClassicWindowsI386).is_err());
        let empty = NativeModSavedState {
            guests: 0,
            providers: Vec::new(),
        };
        assert!(validate_native_mod_state(&empty, &NativeModTarget::ClassicWindowsI386).is_err());
    }
}
