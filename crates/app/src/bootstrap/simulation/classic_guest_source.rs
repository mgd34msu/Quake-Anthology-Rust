//! Classic Quake II guest source: DLL preparation and address-space owner.
//!
//! Port of donor `src/app/bootstrap/simulation/classic-guest-source.ts`
//! (`PreparedClassicGuest`, `prepareClassicGuest`,
//! `ClassicGuestSourceOptions`, `ClassicGuestSource`).
//!
//! The donor wires one address space through CPU, runner, runtime, and
//! host so the DLL executes in place. The Rust pieces take memory by
//! value and the Rust [`ClassicQ2GuestHost`] answers imports from its
//! scripted engine, so the port maps and validates the image in a
//! source-owned address space (export resolution, ABI checks) while the
//! host runs synthetically; CPU/runner/runtime wiring is an explicit
//! wave-2 gap (see the lane report). Consequences: `GetGameAPI` binding
//! moves from `create` to `init` (tests register the handler on the
//! public host first), the engine `services` callback has no host
//! consumer (the world binds services separately), and the import
//! boundary hook is omitted.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use qa_compat::q2::classic::host::ClassicQ2GuestHost;
use qa_compat::q2::classic::layout::{ClassicQ2Error, CLASSIC_Q2_ABI};
use qa_compat::q2::compatibility::{read_native_compatibility, NativeExecution as CompatExecution};
use qa_compat::q2::native_primary::{NativePrimaryDeclaration, PrimaryEdition};
use qa_compat::q2::native_primary_validation::{
    validate_native_primary, PeSection as ValidationSection, SyntheticPeImage,
};
use qa_content::hash::sha256_hex;
use qa_content::mounts::MountError;
use qa_core::identity::ProviderId;
use qa_guest::checkpoint::{GameApi, NativeCallAbi};
use qa_guest::core::contracts::GuestSymbolName;
use qa_guest::core::contracts::{GuestAddress, GuestCallContext, GuestCallbackReference, ModuleIdentity, NativeAbi};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::pe::exports::resolve_pe_export;
use qa_guest::pe::format::parse_pe;
use qa_guest::pe::image::PeImage;
use qa_guest::pe::loader::{map_pe_image, MapPeImageOptions};
use qa_guest::runtime::windows::contracts::{WindowsCapabilities, WindowsFile, WindowsOpenOptions};
use qa_guest::GuestError;
use thiserror::Error;

use super::q2_native_world::{native_module_identity_parts, CompatMounts, GuestSourceMounts};
use crate::persistence::recipe::{ExecutionImplementation, ResolvedExecutionModule};

/// Classic guest source failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ClassicGuestSourceError {
    /// Validation failure with a donor-shaped message.
    #[error("classic guest source: {0}")]
    Invalid(String),
    /// Mount failure.
    #[error("classic guest source mount: {0}")]
    Mount(String),
    /// Guest failure.
    #[error("classic guest source guest: {0}")]
    Guest(String),
    /// Host failure.
    #[error("classic guest source host: {0}")]
    Host(String),
}

impl ClassicGuestSourceError {
    /// Build a validation failure.
    #[must_use]
    pub fn invalid(detail: impl Into<String>) -> Self {
        Self::Invalid(detail.into())
    }
}

impl From<MountError> for ClassicGuestSourceError {
    fn from(error: MountError) -> Self {
        Self::Mount(error.to_string())
    }
}

impl From<GuestError> for ClassicGuestSourceError {
    fn from(error: GuestError) -> Self {
        Self::Guest(error.to_string())
    }
}

impl From<ClassicQ2Error> for ClassicGuestSourceError {
    fn from(error: ClassicQ2Error) -> Self {
        Self::Host(error.to_string())
    }
}

/// Result for classic guest sources.
pub type ClassicGuestSourceResult<T> = Result<T, ClassicGuestSourceError>;

/// Prepared classic guest: the REAL `PreparedClassicGuest` (donor shape;
/// the `types.rs` opaque unifies to it wave 2).
#[derive(Debug, Clone)]
pub struct PreparedClassicGuest {
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

fn compat_execution(
    execution: &ResolvedExecutionModule,
    artifact_path: &str,
    digest: &str,
    api_version: u32,
    profile: NativeAbi,
) -> CompatExecution {
    CompatExecution {
        artifact_path: artifact_path.to_string(),
        digest: digest.to_string(),
        api_version,
        profile,
        owner_content: execution.owner.content.clone(),
    }
}

fn verify_digest(digest: &str, bytes: &[u8]) -> ClassicGuestSourceResult<()> {
    let Some(hex) = digest.strip_prefix("sha256:") else {
        return Err(ClassicGuestSourceError::invalid(
            "Classic native artifact digest is not a sha256 reference",
        ));
    };
    if sha256_hex(bytes) != hex {
        return Err(ClassicGuestSourceError::invalid(
            "Classic native artifact digest differs from the selected module",
        ));
    }
    Ok(())
}

fn synthetic_image(pe: &qa_guest::pe::format::PeFile, pointer_bytes: usize) -> SyntheticPeImage {
    SyntheticPeImage {
        image_size: pe.image_size,
        pointer_bytes,
        sections: pe
            .sections
            .iter()
            .map(|section| ValidationSection {
                rva: section.rva,
                mapped_size: section.mapped_size,
                executable: section.characteristics & 0x2000_0000 != 0,
            })
            .collect(),
    }
}

/// Prepare a classic guest from its execution module and mounts.
pub fn prepare_classic_guest(
    execution: &ResolvedExecutionModule,
    mounts: &dyn GuestSourceMounts,
) -> ClassicGuestSourceResult<PreparedClassicGuest> {
    let invalid = || ClassicGuestSourceError::invalid("Classic Q2 guest requires Windows i386 game API 3");
    if execution.role != "server-game" || execution.api != GameApi::Q2ClassicGame {
        return Err(invalid());
    }
    let Some((artifact, profile)) = native_parts(execution) else {
        return Err(invalid());
    };
    if !matches!(profile, NativeCallAbi::WindowsI386 { .. }) {
        return Err(invalid());
    }
    let bytes = mounts.read_artifact(&artifact.requested_path)?;
    verify_digest(&artifact.digest, &bytes)?;
    let pe = parse_pe(&bytes)?;
    if pe.abi != NativeAbi::WindowsI386 {
        return Err(ClassicGuestSourceError::invalid(
            "Classic native artifact ABI differs from the selected profile",
        ));
    }
    let primary = read_native_compatibility(
        &CompatMounts(mounts),
        &compat_execution(
            execution,
            &artifact.requested_path,
            &artifact.digest,
            3,
            NativeAbi::WindowsI386,
        ),
    );
    if let Some(primary) = &primary {
        validate_native_primary(&primary.profile, &synthetic_image(&pe, 4))
            .map_err(ClassicGuestSourceError::invalid)?;
        if primary.profile.edition() != PrimaryEdition::Classic {
            return Err(ClassicGuestSourceError::invalid(
                "Native declaration edition differs from selected API",
            ));
        }
    }
    Ok(PreparedClassicGuest {
        edition: PrimaryEdition::Classic,
        primary,
        execution: execution.clone(),
        bytes,
    })
}

/// Classic guest source options. The donor's `services(memory)` callback
/// and import boundary have no Rust host consumer; the world binds the
/// services object separately.
pub struct ClassicGuestSourceOptions {
    /// Windows capabilities for guest file access.
    pub capabilities: WindowsCapabilities,
    /// Instruction budget override.
    pub instruction_budget: Option<u64>,
}

/// Tracked open guest files.
#[derive(Default)]
struct SharedFiles {
    next: usize,
    open: HashMap<usize, Box<dyn WindowsFile>>,
}

/// Guest file handle that untracks itself on close.
struct TrackedFile {
    id: usize,
    files: Rc<RefCell<SharedFiles>>,
}

impl WindowsFile for TrackedFile {
    fn read(&mut self, offset: usize, length: usize) -> Vec<u8> {
        self.files
            .borrow_mut()
            .open
            .get_mut(&self.id)
            .expect("tracked file")
            .read(offset, length)
    }

    fn write(&mut self, offset: usize, bytes: &[u8]) -> usize {
        self.files
            .borrow_mut()
            .open
            .get_mut(&self.id)
            .expect("tracked file")
            .write(offset, bytes)
    }

    fn size(&mut self) -> usize {
        self.files
            .borrow_mut()
            .open
            .get_mut(&self.id)
            .expect("tracked file")
            .size()
    }

    fn truncate(&mut self, length: usize) {
        self.files
            .borrow_mut()
            .open
            .get_mut(&self.id)
            .expect("tracked file")
            .truncate(length);
    }

    fn flush(&mut self) {
        self.files
            .borrow_mut()
            .open
            .get_mut(&self.id)
            .expect("tracked file")
            .flush();
    }

    fn close(&mut self) {
        if let Some(mut file) = self.files.borrow_mut().open.remove(&self.id) {
            file.close();
        }
    }
}

/// Source lifecycle phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourcePhase {
    Created,
    Initializing,
    Running,
    Closed,
}

/// The selected DLL owns game bytes; all world and presentation services
/// are supplied by the session.
pub struct ClassicGuestSource {
    /// Synthetic guest host.
    pub host: ClassicQ2GuestHost,
    memory: SparseGuestMemory,
    image: PeImage,
    context: GuestCallContext,
    budget: u64,
    capabilities: WindowsCapabilities,
    files: Rc<RefCell<SharedFiles>>,
    game_export: GuestAddress,
    phase: SourcePhase,
}

impl ClassicGuestSource {
    /// Create a source over a prepared guest.
    pub fn create(
        prepared: PreparedClassicGuest,
        options: ClassicGuestSourceOptions,
    ) -> ClassicGuestSourceResult<Self> {
        if prepared.execution.role != "server-game"
            || prepared.execution.api != GameApi::Q2ClassicGame
            || !matches!(
                native_parts(&prepared.execution).map(|(_, profile)| profile),
                Some(NativeCallAbi::WindowsI386 { .. })
            )
        {
            return Err(ClassicGuestSourceError::invalid(
                "Classic guest source requires selected Windows i386 API 3",
            ));
        }
        let module: ModuleIdentity = native_module_identity_parts(&prepared.execution, prepared.primary.as_ref());
        let mut memory = SparseGuestMemory::new(module.clone(), 4, 0x1_0000)?;
        let cleanup = |memory: &mut SparseGuestMemory| release_memory(memory);
        let image = match map_pe_image(MapPeImageOptions {
            bytes: &prepared.bytes,
            memory: &mut memory,
            module: None,
            base: None,
            maximum_image_bytes: None,
        }) {
            Ok(image) => image,
            Err(error) => {
                let _ = cleanup(&mut memory);
                return Err(error.into());
            }
        };
        let game = match resolve_pe_export(
            &image,
            &GuestSymbolName::Name {
                name: "GetGameAPI".to_string(),
                version: None,
            },
            &|_, _| None,
        ) {
            Ok(export) => export.address,
            Err(error) => {
                let _ = cleanup(&mut memory);
                return Err(error.into());
            }
        };
        let context = GuestCallContext {
            module: module.clone(),
            callback: GuestCallbackReference::NativeGuest {
                module: module.clone(),
                address: game,
                abi: CLASSIC_Q2_ABI,
            },
            parent: None,
            itself: None,
            other: None,
        };
        let budget = options.instruction_budget.unwrap_or(5_000_000);
        let provider = match prepared.execution.owner.provider.split_once(':') {
            Some((namespace, name)) => ProviderId::new(namespace, name),
            None => ProviderId::new("", prepared.execution.owner.provider.as_str()),
        };
        let host = ClassicQ2GuestHost::new(provider, module, budget)?;
        Ok(Self {
            host,
            memory,
            image,
            context,
            budget,
            capabilities: options.capabilities,
            files: Rc::new(RefCell::new(SharedFiles::default())),
            game_export: game,
            phase: SourcePhase::Created,
        })
    }

    /// Guest address space.
    #[must_use]
    pub fn memory(&self) -> &SparseGuestMemory {
        &self.memory
    }

    /// Mutable guest address space.
    pub fn memory_mut(&mut self) -> &mut SparseGuestMemory {
        &mut self.memory
    }

    /// Image base address.
    pub fn image_base(&self) -> GuestAddress {
        self.image.image.base
    }

    /// Resolve an export address.
    pub fn entry(&self, name: &str) -> ClassicGuestSourceResult<GuestAddress> {
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

    /// Open a guest file through the source capabilities, tracking it for
    /// cleanup on discard.
    pub fn open_file(&self, path: &str, options: WindowsOpenOptions) -> Option<Box<dyn WindowsFile>> {
        let opener = self.capabilities.open_file.clone()?;
        let file = opener(path, options)?;
        let mut files = self.files.borrow_mut();
        let id = files.next;
        files.next += 1;
        files.open.insert(id, file);
        drop(files);
        Some(Box::new(TrackedFile {
            id,
            files: Rc::clone(&self.files),
        }))
    }

    /// Initialize the guest (`GetGameAPI` binding happens here; see the
    /// module notes).
    pub fn init(&mut self) -> ClassicGuestSourceResult<()> {
        if self.phase != SourcePhase::Created {
            return Err(ClassicGuestSourceError::invalid(
                "Classic guest Init requires a fresh source",
            ));
        }
        self.phase = SourcePhase::Initializing;
        if let Err(error) = self.host.get_game_api(self.game_export) {
            let message = error.to_string();
            return match self.close() {
                Ok(()) => Err(error.into()),
                Err(cleanup) => Err(ClassicGuestSourceError::invalid(format!(
                    "Classic guest initialization and cleanup failed: {message}; {cleanup}"
                ))),
            };
        }
        match self.host.init() {
            Ok(()) => {
                self.phase = SourcePhase::Running;
                Ok(())
            }
            Err(error) => {
                let message = error.to_string();
                match self.close() {
                    Ok(()) => Err(error.into()),
                    Err(cleanup) => Err(ClassicGuestSourceError::invalid(format!(
                        "Classic guest initialization and cleanup failed: {message}; {cleanup}"
                    ))),
                }
            }
        }
    }

    /// Initialize the guest, yielding to the frame pump around init.
    pub fn init_loading(&mut self, next_frame: &mut dyn FnMut()) -> ClassicGuestSourceResult<()> {
        if self.phase != SourcePhase::Created {
            return Err(ClassicGuestSourceError::invalid(
                "Classic guest Init requires a fresh source",
            ));
        }
        self.phase = SourcePhase::Initializing;
        let bound = self.host.get_game_api(self.game_export);
        if let Err(error) = bound {
            let message = error.to_string();
            return match self.close() {
                Ok(()) => Err(error.into()),
                Err(cleanup) => Err(ClassicGuestSourceError::invalid(format!(
                    "Classic guest initialization and cleanup failed: {message}; {cleanup}"
                ))),
            };
        }
        next_frame();
        match self.host.init() {
            Ok(()) => {
                self.phase = SourcePhase::Running;
                Ok(())
            }
            Err(error) => {
                let message = error.to_string();
                match self.close() {
                    Ok(()) => Err(error.into()),
                    Err(cleanup) => Err(ClassicGuestSourceError::invalid(format!(
                        "Classic guest initialization and cleanup failed: {message}; {cleanup}"
                    ))),
                }
            }
        }
    }

    /// Candidate disposal runs no game exports or DLL detach callbacks.
    pub fn discard(&mut self) -> ClassicGuestSourceResult<()> {
        if self.phase == SourcePhase::Closed {
            return Ok(());
        }
        self.phase = SourcePhase::Closed;
        let mut errors: Vec<String> = Vec::new();
        if let Err(error) = close_files(&self.files) {
            errors.push(error);
        }
        if let Err(error) = release_memory(&mut self.memory) {
            errors.push(error);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(ClassicGuestSourceError::invalid(format!(
                "Classic guest discard failed: {}",
                errors.join("; ")
            )))
        }
    }

    /// Shut the guest down.
    pub fn close(&mut self) -> ClassicGuestSourceResult<()> {
        if self.phase == SourcePhase::Closed {
            return Ok(());
        }
        let initialized = self.phase != SourcePhase::Created;
        let mut errors: Vec<String> = Vec::new();
        if initialized {
            if let Err(error) = self.host.shutdown() {
                errors.push(error.to_string());
            }
        }
        if let Err(error) = self.discard() {
            errors.push(error.to_string());
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(ClassicGuestSourceError::invalid(format!(
                "Classic native guest shutdown failed: {}",
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

fn close_files(files: &Rc<RefCell<SharedFiles>>) -> Result<(), String> {
    let ids: Vec<usize> = files.borrow().open.keys().copied().collect();
    for id in ids {
        if let Some(mut file) = files.borrow_mut().open.remove(&id) {
            file.close();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use qa_guest::checkpoint::{GameApi, NativeCallAbi};
    use qa_guest::runtime::windows::contracts::WindowsCapabilities;

    use super::super::q2_native_world::fixtures::*;
    use super::*;

    fn capabilities() -> WindowsCapabilities {
        WindowsCapabilities {
            now_milliseconds: None,
            performance_counter: None,
            performance_frequency: None,
            command_line: None,
            environment: None,
            open_file: None,
            standard_output: None,
            standard_input: None,
        }
    }

    fn prepared() -> PreparedClassicGuest {
        let mut artifacts = HashMap::new();
        let bytes = minimal_pe(0x14c, 0x10b, &["GetGameAPI"]);
        artifacts.insert("gamex86.dll".to_string(), bytes);
        let mut execution = classic_execution();
        if let ExecutionImplementation::Native { artifact, .. } = &mut execution.implementation {
            artifact.digest = format!("sha256:{}", sha256_hex(&artifacts["gamex86.dll"]));
        }
        let mounts = FakeMounts::new(artifacts, None);
        prepare_classic_guest(&execution, &mounts).expect("prepared")
    }

    #[test]
    fn prepare_validates_execution() {
        let mounts = FakeMounts::new(HashMap::new(), None);
        let mut bad = classic_execution();
        bad.role = "client-game".to_string();
        assert!(prepare_classic_guest(&bad, &mounts).is_err());
        let mut bad = classic_execution();
        bad.api = GameApi::Q2RereleaseGame;
        assert!(prepare_classic_guest(&bad, &mounts).is_err());
        let mut bad = classic_execution();
        bad.implementation = ExecutionImplementation::Builtin {
            implementation: "x".to_string(),
        };
        assert!(prepare_classic_guest(&bad, &mounts).is_err());
        let mut bad = classic_execution();
        if let ExecutionImplementation::Native { profile, .. } = &mut bad.implementation {
            *profile = NativeCallAbi::WindowsX8664;
        }
        assert!(prepare_classic_guest(&bad, &mounts).is_err());
    }

    #[test]
    fn prepare_reads_and_validates_artifact() {
        let guest = prepared();
        assert_eq!(guest.edition, PrimaryEdition::Classic);
        assert!(guest.primary.is_none());
        assert!(!guest.bytes.is_empty());
    }

    #[test]
    fn prepare_rejects_digest_mismatch() {
        let mut artifacts = HashMap::new();
        artifacts.insert("gamex86.dll".to_string(), minimal_pe(0x14c, 0x10b, &["GetGameAPI"]));
        let mounts = FakeMounts::new(artifacts, None);
        let execution = classic_execution();
        assert!(prepare_classic_guest(&execution, &mounts).is_err());
    }

    #[test]
    fn create_maps_image_and_resolves_entry() {
        let guest = prepared();
        let mut source = ClassicGuestSource::create(
            guest,
            ClassicGuestSourceOptions {
                capabilities: capabilities(),
                instruction_budget: None,
            },
        )
        .expect("source");
        assert_eq!(source.budget(), 5_000_000);
        let entry = source.entry("GetGameAPI").expect("entry");
        assert_eq!(entry.offset - source.image_base().offset, 0x1100);
        source.close().expect("close");
    }

    #[test]
    fn lifecycle_guards_phases() {
        let guest = prepared();
        let mut source = ClassicGuestSource::create(
            guest,
            ClassicGuestSourceOptions {
                capabilities: capabilities(),
                instruction_budget: Some(10),
            },
        )
        .expect("source");
        assert_eq!(source.budget(), 10);
        // No GetGameAPI handler is registered, so init fails and closes.
        assert!(source.init().is_err());
        assert!(source.init().is_err());
        source.close().expect("close-again");
    }

    #[test]
    fn open_file_returns_none_without_opener() {
        let guest = prepared();
        let source = ClassicGuestSource::create(
            guest,
            ClassicGuestSourceOptions {
                capabilities: capabilities(),
                instruction_budget: None,
            },
        )
        .expect("source");
        assert!(source
            .open_file(
                "x",
                WindowsOpenOptions {
                    read: true,
                    write: false,
                    creation: 3
                }
            )
            .is_none());
    }
}
