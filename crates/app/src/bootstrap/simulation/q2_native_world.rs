//! Quake II native world union and prepared-guest identity.
//!
//! Port of donor `src/app/bootstrap/simulation/q2-native-world.ts`
//! (`Q2NativeWorld`, `PreparedQ2NativeGuest`, `nativeModuleIdentity`,
//! `preparedNativePrimary`).

use qa_compat::q2::compatibility::{CompatibilityMounts, CompatibilityResource};
use qa_compat::q2::native_primary::{
    builtin_native_primary, NativePrimaryDeclaration, NativePrimaryProfile, PrimaryEdition,
};
use qa_content::hash::sha256_hex;
use qa_content::mounts::{MountedContent, ResourceRef};
use qa_core::identity::ProviderId;
use qa_guest::core::contracts::{ContentDigest, ModuleIdentity};

use super::classic_guest_source::PreparedClassicGuest;
use super::classic_guest_world::ClassicGuestWorld;
use super::rerelease_guest_source::PreparedRereleaseGuest;
use super::rerelease_guest_world::RereleaseGuestWorld;
use crate::persistence::recipe::{ExecutionImplementation, ResolvedExecutionModule};

/// Prepared classic or rerelease native guest.
#[derive(Debug, Clone)]
pub enum PreparedQ2NativeGuest {
    /// Classic guest.
    Classic(PreparedClassicGuest),
    /// Rerelease guest.
    Rerelease(PreparedRereleaseGuest),
}

/// Mount access for native guest preparation: artifact bytes plus the
/// compatibility document.
///
/// The donor takes the concrete `MountedContent`; the trait keeps file
/// access injectable for tests. The mount plan is already scoped to the
/// selected content, so the compatibility lookup ignores its owner filter.
pub trait GuestSourceMounts {
    /// Read an artifact by requested path.
    fn read_artifact(&self, path: &str) -> Result<Vec<u8>, qa_content::mounts::MountError>;
    /// Open the compatibility document, if present.
    fn open_compat_doc(&self) -> Option<CompatibilityResource>;
}

impl GuestSourceMounts for MountedContent {
    fn read_artifact(&self, path: &str) -> Result<Vec<u8>, qa_content::mounts::MountError> {
        self.read(ResourceRef::Path(path))
    }

    fn open_compat_doc(&self) -> Option<CompatibilityResource> {
        match self.read(ResourceRef::Path("native-compatibility.json")) {
            Ok(bytes) => Some(CompatibilityResource {
                bytes,
                reference: "native-compatibility.json".to_string(),
            }),
            Err(_) => None,
        }
    }
}

/// Adapter from guest source mounts to the compatibility lookup.
pub(crate) struct CompatMounts<'a>(pub(crate) &'a dyn GuestSourceMounts);

impl CompatibilityMounts for CompatMounts<'_> {
    fn open_native_compatibility(&self, _owner_content: &str) -> Option<CompatibilityResource> {
        self.0.open_compat_doc()
    }
}

fn provider_id(reference: &str) -> ProviderId {
    match reference.split_once(':') {
        Some((namespace, name)) => ProviderId::new(namespace, name),
        None => ProviderId::new("", reference),
    }
}

fn content_digest(digest: &str) -> ContentDigest {
    match digest.split_once(':') {
        Some((algorithm, value)) => ContentDigest::new(algorithm, value),
        None => ContentDigest::new("unknown", digest),
    }
}

fn native_artifact(execution: &ResolvedExecutionModule) -> (String, String) {
    match &execution.implementation {
        ExecutionImplementation::Native { artifact, .. } => {
            (artifact.requested_path.clone(), artifact.identity.clone())
        }
        _ => (String::new(), String::new()),
    }
}

/// Module identity shared by the native execution.
///
/// The donor revision is `digest/declaration-path/declaration-digest`; Rust
/// declaration references are flat strings, so the revision is
/// `digest/declaration-reference`.
pub fn native_module_identity_parts(
    execution: &ResolvedExecutionModule,
    primary: Option<&NativePrimaryDeclaration>,
) -> ModuleIdentity {
    let (requested_path, identity) = native_artifact(execution);
    ModuleIdentity::new(
        provider_id(&execution.owner.provider),
        &requested_path,
        content_digest(&identity),
        &match primary {
            None => identity.clone(),
            Some(primary) => format!("{identity}/{}", primary.declaration),
        },
    )
}

/// Module identity for a prepared native guest.
#[must_use]
pub fn native_module_identity(prepared: &PreparedQ2NativeGuest) -> ModuleIdentity {
    match prepared {
        PreparedQ2NativeGuest::Classic(prepared) => {
            native_module_identity_parts(&prepared.execution, prepared.primary.as_ref())
        }
        PreparedQ2NativeGuest::Rerelease(prepared) => {
            native_module_identity_parts(&prepared.execution, prepared.primary.as_ref())
        }
    }
}

/// Declared or builtin primary profile for a prepared guest.
///
/// The donor memoizes builtin lookups per guest; the lookup is pure, so the
/// port recomputes it.
#[must_use]
pub fn prepared_native_primary(prepared: &PreparedQ2NativeGuest) -> Option<NativePrimaryProfile> {
    match prepared {
        PreparedQ2NativeGuest::Classic(prepared) => match &prepared.primary {
            Some(primary) => Some(primary.profile.clone()),
            None => {
                let digest = format!("sha256:{}", sha256_hex(&prepared.bytes));
                builtin_native_primary(&digest, PrimaryEdition::Classic)
            }
        },
        PreparedQ2NativeGuest::Rerelease(prepared) => match &prepared.primary {
            Some(primary) => Some(primary.profile.clone()),
            None => {
                let digest = format!("sha256:{}", sha256_hex(&prepared.bytes));
                builtin_native_primary(&digest, PrimaryEdition::Rerelease)
            }
        },
    }
}

/// Classic or rerelease native guest world (donor `Q2NativeWorld`).
pub enum Q2NativeWorld {
    /// Classic world.
    Classic(Box<ClassicGuestWorld>),
    /// Rerelease world.
    Rerelease(Box<RereleaseGuestWorld>),
}

impl Q2NativeWorld {
    /// World edition tag.
    #[must_use]
    pub fn edition(&self) -> &'static str {
        match self {
            Self::Classic(_) => "classic",
            Self::Rerelease(_) => "rerelease",
        }
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use std::collections::HashMap;

    use qa_content::mounts::MountError;
    use qa_guest::checkpoint::{GameApi, NativeCall, NativeCallAbi};

    use super::*;
    use crate::persistence::recipe::{
        ContentMount, MountIdentity, ResolvedResourceReference, ResourceProvenance, ResourceResolution,
    };

    /// Native test execution module.
    pub(crate) fn test_execution(
        api: GameApi,
        profile: NativeCallAbi,
        requested_path: &str,
        identity: &str,
    ) -> ResolvedExecutionModule {
        ResolvedExecutionModule {
            owner: qa_world::save::shared::ProviderRef {
                provider: "q2:test".to_string(),
                content: "q2:test:baseq2:1".to_string(),
            },
            role: "server-game".to_string(),
            api,
            implementation: ExecutionImplementation::Native {
                artifact: ResolvedResourceReference {
                    id: "game".to_string(),
                    requested_path: requested_path.to_string(),
                    provenance: ResourceProvenance::Loose {
                        mount: Box::new(ContentMount::Loose {
                            identity: MountIdentity {
                                id: "m".to_string(),
                                content: "q2:test:baseq2:1".to_string(),
                                generation: 1,
                            },
                            root_path: "/tmp".to_string(),
                        }),
                        member_path: requested_path.to_string(),
                    },
                    identity: identity.to_string(),
                    byte_length: 0,
                    resolution: ResourceResolution::Link {
                        plan: "p".to_string(),
                        source_prefix: String::new(),
                        target_path: requested_path.to_string(),
                    },
                },
                profile,
            },
        }
    }

    /// Classic i386 test execution.
    pub(crate) fn classic_execution() -> ResolvedExecutionModule {
        test_execution(
            GameApi::Q2ClassicGame,
            NativeCallAbi::WindowsI386 {
                call: NativeCall::Cdecl,
            },
            "gamex86.dll",
            "identity:1:2:3:4",
        )
    }

    /// Rerelease x64 test execution.
    pub(crate) fn rerelease_execution() -> ResolvedExecutionModule {
        test_execution(
            GameApi::Q2RereleaseGame,
            NativeCallAbi::WindowsX8664,
            "q2game.dll",
            "identity:5:6:7:8",
        )
    }

    fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    /// Minimal single-section PE exporting `exports` from one code section.
    /// `machine`/`magic` select i386 (`0x14c`/`0x10b`) or x64
    /// (`0x8664`/`0x20b`).
    pub(crate) fn minimal_pe(machine: u16, magic: u16, exports: &[&str]) -> Vec<u8> {
        let mut bytes = vec![0u8; 0x400];
        put_u16(&mut bytes, 0, 0x5a4d);
        put_u32(&mut bytes, 0x3c, 64);
        bytes[64..68].copy_from_slice(b"PE\0\0");
        put_u16(&mut bytes, 68, machine);
        put_u16(&mut bytes, 70, 1);
        let optional_size = if magic == 0x10b { 224 } else { 240 };
        put_u16(&mut bytes, 84, optional_size as u16);
        put_u16(&mut bytes, 86, 2);
        let optional = 88usize;
        put_u16(&mut bytes, optional, magic);
        put_u32(&mut bytes, optional + 16, 0x1000);
        if magic == 0x10b {
            put_u32(&mut bytes, optional + 28, 0x400000);
        } else {
            put_u64(&mut bytes, optional + 24, 0x280000000);
        }
        put_u32(&mut bytes, optional + 32, 0x1000);
        put_u32(&mut bytes, optional + 36, 0x200);
        put_u32(&mut bytes, optional + 56, 0x2000);
        put_u32(&mut bytes, optional + 60, 0x200);
        let directory_offset = if magic == 0x10b { 96 } else { 112 };
        put_u32(&mut bytes, optional + directory_offset - 4, 1);
        put_u32(&mut bytes, optional + directory_offset, 0x1000);
        put_u32(&mut bytes, optional + directory_offset + 4, 0x100);
        let table = optional + optional_size;
        bytes[table..table + 5].copy_from_slice(b".text");
        put_u32(&mut bytes, table + 8, 0x200);
        put_u32(&mut bytes, table + 12, 0x1000);
        put_u32(&mut bytes, table + 16, 0x200);
        put_u32(&mut bytes, table + 20, 0x200);
        put_u32(&mut bytes, table + 36, 0x60000020);
        // Export directory at RVA 0x1000 (file 0x200).
        let dir = 0x200usize;
        let count = exports.len() as u32;
        put_u32(&mut bytes, dir + 12, 0x1080);
        put_u32(&mut bytes, dir + 16, 1);
        put_u32(&mut bytes, dir + 20, count);
        put_u32(&mut bytes, dir + 24, count);
        put_u32(&mut bytes, dir + 28, 0x1040);
        put_u32(&mut bytes, dir + 32, 0x1060);
        put_u32(&mut bytes, dir + 36, 0x1070);
        bytes[0x280..0x28b].copy_from_slice(b"gamex86.dll");
        let mut name_rva = 0x1090u32;
        for (index, name) in exports.iter().enumerate() {
            put_u32(&mut bytes, 0x240 + index * 4, 0x1100 + index as u32 * 0x10);
            put_u32(&mut bytes, 0x260 + index * 4, name_rva);
            put_u16(&mut bytes, 0x270 + index * 2, index as u16);
            let at = 0x200 + (name_rva - 0x1000) as usize;
            bytes[at..at + name.len()].copy_from_slice(name.as_bytes());
            name_rva += name.len() as u32 + 1;
        }
        bytes[0x300] = 0xc3;
        bytes
    }

    /// In-memory mounts for source preparation tests.
    pub(crate) struct FakeMounts {
        artifacts: HashMap<String, Vec<u8>>,
        compat: Option<Vec<u8>>,
    }

    impl FakeMounts {
        pub(crate) fn new(artifacts: HashMap<String, Vec<u8>>, compat: Option<Vec<u8>>) -> Self {
            Self { artifacts, compat }
        }
    }

    impl GuestSourceMounts for FakeMounts {
        fn read_artifact(&self, path: &str) -> Result<Vec<u8>, MountError> {
            self.artifacts
                .get(path)
                .cloned()
                .ok_or_else(|| MountError::Failed(format!("Resource not found: {path}")))
        }

        fn open_compat_doc(&self) -> Option<CompatibilityResource> {
            self.compat.clone().map(|bytes| CompatibilityResource {
                bytes,
                reference: "native-compatibility.json".to_string(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use qa_compat::q2::native_primary_reader::CLASSIC_DIGEST;

    use super::fixtures::*;
    use super::*;

    #[test]
    fn identity_uses_artifact_identity_as_revision_without_primary() {
        let prepared = PreparedQ2NativeGuest::Classic(PreparedClassicGuest {
            edition: PrimaryEdition::Classic,
            primary: None,
            execution: classic_execution(),
            bytes: Vec::new(),
        });
        let module = native_module_identity(&prepared);
        assert_eq!(module.artifact_path, "gamex86.dll");
        assert_eq!(module.digest, ContentDigest::new("identity", "1:2:3:4"));
        assert_eq!(module.revision, "identity:1:2:3:4");
        assert_eq!(module.id, ProviderId::new("q2", "test"));
    }

    #[test]
    fn identity_appends_declaration_reference_with_primary() {
        let profile = builtin_native_primary(CLASSIC_DIGEST, PrimaryEdition::Classic).expect("builtin");
        let prepared = PreparedQ2NativeGuest::Classic(PreparedClassicGuest {
            edition: PrimaryEdition::Classic,
            primary: Some(NativePrimaryDeclaration {
                declaration: "native-compatibility.json".to_string(),
                profile: profile.clone(),
            }),
            execution: classic_execution(),
            bytes: Vec::new(),
        });
        let module = native_module_identity(&prepared);
        assert_eq!(module.revision, "identity:1:2:3:4/native-compatibility.json");
        assert_eq!(prepared_native_primary(&prepared), Some(profile));
    }

    #[test]
    fn builtin_primary_rejects_unknown_bytes() {
        let execution = classic_execution();
        let prepared = PreparedQ2NativeGuest::Classic(PreparedClassicGuest {
            edition: PrimaryEdition::Classic,
            primary: None,
            execution,
            bytes: Vec::new(),
        });
        assert!(prepared_native_primary(&prepared).is_none());
        assert!(
            prepared_native_primary(&PreparedQ2NativeGuest::Rerelease(PreparedRereleaseGuest {
                edition: PrimaryEdition::Rerelease,
                primary: None,
                execution: rerelease_execution(),
                bytes: Vec::new(),
            }))
            .is_none()
        );
    }
}
