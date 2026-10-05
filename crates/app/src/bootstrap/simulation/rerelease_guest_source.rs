//! Rerelease Quake II guest source: DLL preparation and address-space owner.
//!
//! Port of donor `src/app/bootstrap/simulation/rerelease-guest-source.ts`
//! (`PreparedRereleaseGuest`, `prepareRereleaseGuest`,
//! `RereleaseGuestSourceOptions`, `RereleaseGuestSource`).
//!
//! Like the classic source, the port maps and validates the image while
//! the Rust [`RereleaseQ2GuestHost`] runs synthetically; CPU/runner/
//! runtime wiring is an explicit wave-2 gap (see the lane report). The
//! mapped address space moves into the host at construction, so entity
//! reads observe the real image bytes. The donor's `services(memory)`
//! callback and engine/spatial/semantics/message/sound/debug options
//! have no host consumer: the world binds the services object
//! separately, and the source keeps only the clock, budget, foreign
//! damage, pickups, and import interception.

use qa_compat::q2::compatibility::{read_native_compatibility, NativeExecution as CompatExecution};
use qa_compat::q2::native_primary::{rerelease_primary_world_profile, NativePrimaryDeclaration, PrimaryEdition};
use qa_compat::q2::native_primary_validation::{
    validate_native_primary, PeSection as ValidationSection, SyntheticPeImage,
};
use qa_compat::q2::rerelease::api::rerelease_abi;
use qa_compat::q2::rerelease::host::{HostOptions, RereleaseQ2GuestHost};
use qa_content::archive::crc32;
use qa_content::contract::ResourceIdentity;
use qa_content::hash::sha256_hex;
use qa_content::mounts::MountError;
use qa_guest::checkpoint::{GameApi, NativeCallAbi};
use qa_guest::core::contracts::{
    GuestAddress, GuestCallContext, GuestCallResult, GuestCallValue, GuestCallbackReference, GuestSymbolName,
    ModuleIdentity, NativeAbi,
};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::pe::exports::resolve_pe_export;
use qa_guest::pe::format::parse_pe;
use qa_guest::pe::image::PeImage;
use qa_guest::pe::loader::{map_pe_image, MapPeImageOptions};
use qa_guest::GuestError;
use thiserror::Error;

use super::classic_guest_services::{ClassicDamageProvenance, GuestPickupAdmission};
use super::q2_native_world::{native_module_identity_parts, CompatMounts, GuestSourceMounts};
use crate::persistence::recipe::{ExecutionImplementation, ResolvedExecutionModule};

/// Rerelease guest source failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RereleaseGuestSourceError {
    /// Validation failure with a donor-shaped message.
    #[error("rerelease guest source: {0}")]
    Invalid(String),
    /// Mount failure.
    #[error("rerelease guest source mount: {0}")]
    Mount(String),
    /// Guest failure.
    #[error("rerelease guest source guest: {0}")]
    Guest(String),
    /// Host failure.
    #[error("rerelease guest source host: {0}")]
    Host(String),
}

impl RereleaseGuestSourceError {
    /// Build a validation failure.
    #[must_use]
    pub fn invalid(detail: impl Into<String>) -> Self {
        Self::Invalid(detail.into())
    }
}

impl From<MountError> for RereleaseGuestSourceError {
    fn from(error: MountError) -> Self {
        Self::Mount(error.to_string())
    }
}

impl From<GuestError> for RereleaseGuestSourceError {
    fn from(error: GuestError) -> Self {
        Self::Guest(error.to_string())
    }
}

/// Result for rerelease guest sources.
pub type RereleaseGuestSourceResult<T> = Result<T, RereleaseGuestSourceError>;

/// Prepared rerelease guest: the REAL `PreparedRereleaseGuest` (donor
/// shape; the `types.rs` opaque unifies to it wave 2).
#[derive(Debug, Clone)]
pub struct PreparedRereleaseGuest {
    /// Source edition tag.
    pub edition: PrimaryEdition,
    /// Declared primary, when the compatibility document names one.
    pub primary: Option<NativePrimaryDeclaration>,
    /// Native execution module.
    pub execution: ResolvedExecutionModule,
    /// Artifact bytes.
    pub bytes: Vec<u8>,
}

fn native_parts(
    execution: &ResolvedExecutionModule,
) -> Option<(&crate::persistence::recipe::ResolvedResourceReference, &NativeCallAbi)> {
    match &execution.implementation {
        ExecutionImplementation::Native { artifact, profile } => Some((artifact, profile)),
        _ => None,
    }
}

/// Prepare a rerelease guest from its execution module and mounts.
pub fn prepare_rerelease_guest(
    execution: &ResolvedExecutionModule,
    mounts: &dyn GuestSourceMounts,
) -> RereleaseGuestSourceResult<PreparedRereleaseGuest> {
    if execution.role != "server-game"
        || execution.api != GameApi::Q2RereleaseGame
        || !matches!(
            native_parts(execution).map(|(_, profile)| profile),
            Some(NativeCallAbi::WindowsX8664)
        )
    {
        return Err(RereleaseGuestSourceError::invalid(
            "Rerelease guest requires the native Windows x64 game API 2023",
        ));
    }
    let (artifact, _) = native_parts(execution).expect("native execution");
    let bytes = mounts.read_artifact(&artifact.requested_path)?;
    match ResourceIdentity::parse(&artifact.identity) {
        Some(expected) if expected.byte_length == bytes.len() as u64 && expected.crc == crc32(&bytes) => {}
        _ => {
            return Err(RereleaseGuestSourceError::invalid(
                "Rerelease native artifact bytes differ from the selected module",
            ));
        }
    }
    let digest = format!("sha256:{}", sha256_hex(&bytes));
    let pe = parse_pe(&bytes)?;
    if pe.abi != NativeAbi::WindowsX86_64 {
        return Err(RereleaseGuestSourceError::invalid(
            "Native artifact ABI differs from the selected profile",
        ));
    }
    let primary = read_native_compatibility(
        &CompatMounts(mounts),
        &CompatExecution {
            artifact_path: artifact.requested_path.clone(),
            digest,
            api_version: 2023,
            profile: NativeAbi::WindowsX86_64,
            owner_content: execution.owner.content.clone(),
        },
    );
    if let Some(primary) = &primary {
        validate_native_primary(
            &primary.profile,
            &SyntheticPeImage {
                image_size: pe.image_size,
                pointer_bytes: 8,
                sections: pe
                    .sections
                    .iter()
                    .map(|section| ValidationSection {
                        rva: section.rva,
                        mapped_size: section.mapped_size,
                        executable: section.characteristics & 0x2000_0000 != 0,
                    })
                    .collect(),
            },
        )
        .map_err(RereleaseGuestSourceError::invalid)?;
        if primary.profile.edition() != PrimaryEdition::Rerelease {
            return Err(RereleaseGuestSourceError::invalid(
                "Native declaration edition differs from selected API",
            ));
        }
    }
    Ok(PreparedRereleaseGuest {
        edition: PrimaryEdition::Rerelease,
        primary,
        execution: execution.clone(),
        bytes,
    })
}

/// Guest clock capabilities (mirror of `Required<Pick<WindowsCapabilities,
/// 'nowMilliseconds' | 'performanceCounter' | 'performanceFrequency'>>`).
#[derive(Clone)]
pub struct GuestClock {
    /// Milliseconds since the epoch.
    pub now_milliseconds: std::rc::Rc<dyn Fn() -> i64>,
    /// Performance counter value.
    pub performance_counter: std::rc::Rc<dyn Fn() -> u64>,
    /// Performance counter frequency.
    pub performance_frequency: u64,
}

impl std::fmt::Debug for GuestClock {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GuestClock")
            .field("performance_frequency", &self.performance_frequency)
            .finish_non_exhaustive()
    }
}

/// Import interception: `(api, name, arguments) -> scripted result`.
pub type InterceptImport = Box<dyn FnMut(&str, &str, &[GuestCallValue]) -> Option<GuestCallResult>>;

/// Rerelease guest source options.
pub struct RereleaseGuestSourceOptions {
    /// Guest clock.
    pub clock: GuestClock,
    /// Instruction budget override.
    pub instruction_budget: Option<u64>,
    /// Foreign damage provenance.
    pub foreign_damage: Option<ClassicDamageProvenance>,
    /// Native pickup admission.
    pub pickups: Option<GuestPickupAdmission>,
    /// Import interception.
    pub intercept_import: Option<InterceptImport>,
}

/// Owns only the guest address space and ABI lifetime; supplied engine
/// authorities own the world.
pub struct RereleaseGuestSource {
    /// Synthetic guest host.
    pub host: RereleaseQ2GuestHost,
    image: PeImage,
    context: GuestCallContext,
    budget: u64,
    clock: GuestClock,
    pickups: Option<GuestPickupAdmission>,
    game_export: GuestAddress,
    cgame_export: GuestAddress,
    closed: bool,
}

impl RereleaseGuestSource {
    /// Create a source over a prepared guest.
    pub fn create(
        prepared: PreparedRereleaseGuest,
        options: RereleaseGuestSourceOptions,
    ) -> RereleaseGuestSourceResult<Self> {
        let module: ModuleIdentity = native_module_identity_parts(&prepared.execution, prepared.primary.as_ref());
        let mut memory = SparseGuestMemory::new(module.clone(), 8, 0x1_0000)?;
        let image = map_pe_image(MapPeImageOptions {
            bytes: &prepared.bytes,
            memory: &mut memory,
            module: None,
            base: Some(0x280000000),
            maximum_image_bytes: None,
        })?;
        let export = |image: &PeImage, name: &str| {
            resolve_pe_export(
                image,
                &GuestSymbolName::Name {
                    name: name.to_string(),
                    version: None,
                },
                &|_, _| None,
            )
            .map(|export| export.address)
        };
        let game = export(&image, "GetGameAPI")?;
        let cgame = export(&image, "GetCGameAPI")?;
        let context = GuestCallContext {
            module: module.clone(),
            callback: GuestCallbackReference::NativeGuest {
                module: module.clone(),
                address: game,
                abi: rerelease_abi(),
            },
            parent: None,
            itself: None,
            other: None,
        };
        let budget = options.instruction_budget.unwrap_or(5_000_000);
        let has_world_profile = match &prepared.primary {
            Some(primary) => matches!(
                primary.profile,
                qa_compat::q2::native_primary::NativePrimaryProfile::Rerelease { .. }
            ),
            None => {
                let digest = match &prepared.execution.implementation {
                    ExecutionImplementation::Native { .. } => {
                        format!("sha256:{}", sha256_hex(&prepared.bytes))
                    }
                    _ => String::new(),
                };
                rerelease_primary_world_profile(&digest).is_some()
            }
        };
        if options.foreign_damage.is_some() && !has_world_profile {
            return Err(RereleaseGuestSourceError::invalid(
                "Original native damage requires a declared source world profile",
            ));
        }
        let foreign = options.foreign_damage.is_some();
        let mut host = RereleaseQ2GuestHost::new(
            memory,
            &HostOptions {
                pickups_admitted: options.pickups.is_some(),
                component_projection: false,
                headless_debug: false,
                debug_shapes_bound: false,
                world_text_bound: false,
                foreign_damage: foreign,
                native_entries: foreign,
            },
        )
        .map_err(|error| RereleaseGuestSourceError::Host(error.to_string()))?;
        if let Some(intercept) = options.intercept_import {
            host.set_intercept(intercept);
        }
        Ok(Self {
            host,
            image,
            context,
            budget,
            clock: options.clock,
            pickups: options.pickups,
            game_export: game,
            cgame_export: cgame,
            closed: false,
        })
    }

    /// Image base address.
    #[must_use]
    pub fn image_base(&self) -> GuestAddress {
        self.image.image.base
    }

    /// Resolve an export address.
    pub fn entry(&self, name: &str) -> RereleaseGuestSourceResult<GuestAddress> {
        Ok(resolve_pe_export(
            &self.image,
            &GuestSymbolName::Name {
                name: name.to_string(),
                version: None,
            },
            &|_, _| None,
        )?
        .address)
    }

    /// Guest call context.
    #[must_use]
    pub fn context(&self) -> &GuestCallContext {
        &self.context
    }

    /// Instruction budget.
    #[must_use]
    pub fn budget(&self) -> u64 {
        self.budget
    }

    /// Guest clock (retained for runtime wiring).
    #[must_use]
    pub fn clock(&self) -> &GuestClock {
        &self.clock
    }

    /// Whether pickup admission was declared.
    #[must_use]
    pub fn has_pickups(&self) -> bool {
        self.pickups.is_some()
    }

    /// `GetGameAPI` export address.
    #[must_use]
    pub fn game_export(&self) -> GuestAddress {
        self.game_export
    }

    /// `GetCGameAPI` export address.
    #[must_use]
    pub fn cgame_export(&self) -> GuestAddress {
        self.cgame_export
    }

    /// Initialize the guest.
    pub fn init(&mut self) -> RereleaseGuestSourceResult<()> {
        if self.closed {
            return Err(RereleaseGuestSourceError::invalid("Native guest source is closed"));
        }
        match self.host.init() {
            Ok(()) => Ok(()),
            Err(error) => {
                let message = error.to_string();
                match self.close() {
                    Ok(()) => Err(RereleaseGuestSourceError::Host(message)),
                    Err(cleanup) => Err(RereleaseGuestSourceError::invalid(format!(
                        "Native guest initialization and cleanup failed: {message}; {cleanup}"
                    ))),
                }
            }
        }
    }

    /// Initialize the guest, yielding to the frame pump around init.
    pub fn init_loading(&mut self, next_frame: &mut dyn FnMut()) -> RereleaseGuestSourceResult<()> {
        if self.closed {
            return Err(RereleaseGuestSourceError::invalid("Native guest source is closed"));
        }
        next_frame();
        match self.host.init() {
            Ok(()) => Ok(()),
            Err(error) => {
                let message = error.to_string();
                match self.close() {
                    Ok(()) => Err(RereleaseGuestSourceError::Host(message)),
                    Err(cleanup) => Err(RereleaseGuestSourceError::invalid(format!(
                        "Native guest initialization and cleanup failed: {message}; {cleanup}"
                    ))),
                }
            }
        }
    }

    /// Shut the guest down.
    pub fn close(&mut self) -> RereleaseGuestSourceResult<()> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        let mut errors: Vec<String> = Vec::new();
        self.host.shutdown();
        if let Err(error) = release_memory(&mut self.host.memory) {
            errors.push(error);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(RereleaseGuestSourceError::invalid(format!(
                "Native guest shutdown failed: {}",
                errors.join("; ")
            )))
        }
    }
}

fn release_memory(memory: &mut SparseGuestMemory) -> Result<(), String> {
    for mapping in memory.mappings() {
        let address = memory.pointer(mapping.base).map_err(|error| error.to_string())?;
        if let Some(address) = address {
            memory
                .unmap(address, mapping.byte_length)
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::rc::Rc;

    use qa_guest::checkpoint::{GameApi, NativeCallAbi};

    use super::super::q2_native_world::fixtures::*;
    use super::*;
    use crate::persistence::recipe::ExecutionImplementation;

    fn clock() -> GuestClock {
        GuestClock {
            now_milliseconds: Rc::new(|| 0),
            performance_counter: Rc::new(|| 0),
            performance_frequency: 1,
        }
    }

    fn prepared() -> PreparedRereleaseGuest {
        let mut artifacts = HashMap::new();
        let bytes = minimal_pe(0x8664, 0x20b, &["GetGameAPI", "GetCGameAPI"]);
        artifacts.insert("q2game.dll".to_string(), bytes);
        let mut execution = rerelease_execution();
        if let ExecutionImplementation::Native { artifact, .. } = &mut execution.implementation {
            let image = &artifacts["q2game.dll"];
            artifact.identity = ResourceIdentity {
                mount_generation: 0,
                member_index: 0,
                byte_length: image.len() as u64,
                crc: crc32(image),
            }
            .canonical();
        }
        let mounts = FakeMounts::new(artifacts, None);
        prepare_rerelease_guest(&execution, &mounts).expect("prepared")
    }

    fn options() -> RereleaseGuestSourceOptions {
        RereleaseGuestSourceOptions {
            clock: clock(),
            instruction_budget: None,
            foreign_damage: None,
            pickups: None,
            intercept_import: None,
        }
    }

    #[test]
    fn prepare_validates_execution() {
        let mounts = FakeMounts::new(HashMap::new(), None);
        let mut bad = rerelease_execution();
        bad.role = "client-game".to_string();
        assert!(prepare_rerelease_guest(&bad, &mounts).is_err());
        let mut bad = rerelease_execution();
        bad.api = GameApi::Q2ClassicGame;
        assert!(prepare_rerelease_guest(&bad, &mounts).is_err());
        let mut bad = rerelease_execution();
        if let ExecutionImplementation::Native { profile, .. } = &mut bad.implementation {
            *profile = NativeCallAbi::WindowsI386 {
                call: qa_guest::checkpoint::NativeCall::Cdecl,
            };
        }
        assert!(prepare_rerelease_guest(&bad, &mounts).is_err());
    }

    #[test]
    fn create_maps_image_and_resolves_entries() {
        let guest = prepared();
        let mut source = RereleaseGuestSource::create(guest, options()).expect("source");
        assert_eq!(source.budget(), 5_000_000);
        assert_eq!(source.game_export().offset - source.image_base().offset, 0x1100);
        assert_eq!(source.cgame_export().offset - source.image_base().offset, 0x1110);
        assert_eq!(source.entry("GetGameAPI").expect("entry"), source.game_export());
        source.init().expect("init");
        source.close().expect("close");
        source.close().expect("close-again");
    }

    #[test]
    fn foreign_damage_requires_world_profile() {
        let guest = prepared();
        let mut with_foreign = options();
        with_foreign.foreign_damage = Some(Rc::new(|_, _, _| panic!("provenance is validated, never invoked here")));
        assert!(RereleaseGuestSource::create(guest, with_foreign).is_err());
    }

    #[test]
    fn init_loading_yields_and_initializes() {
        let guest = prepared();
        let mut source = RereleaseGuestSource::create(guest, options()).expect("source");
        let mut pumps = 0;
        source.init_loading(&mut || pumps += 1).expect("init");
        assert_eq!(pumps, 1);
        source.close().expect("close");
    }
}
