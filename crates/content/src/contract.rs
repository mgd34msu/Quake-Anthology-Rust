//! Shared content contract types absorbed from `src/contracts`.
//!
//! Donors: `content.ts`, `mods.ts`, `equipment.ts`, `held-weapon.ts`,
//! `model-attachment.ts`, `pickups.ts`, `original-pickups.ts`,
//! `mod-callbacks.ts`, `native-mod-callbacks.ts`, `native-mod-items.ts`,
//! `native-mod-region.ts`, `source-items.ts`, `presentation.ts`,
//! `mod-client-outputs.ts`, `source-match.ts`. Identity handles live in
//! [`qa_core::identity`] (donor `identity.ts`). Types referenced from
//! out-of-scope contracts are defined here structurally with their donor
//! noted, since `qa-content` depends only on `qa-core`.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::fmt::{self, Display};
use std::rc::{Rc, Weak};

use qa_core::cmd::{command_text_tail, tokenize_command, Dialect, TextMode};
use qa_core::cmd_buffer::CommandOrigin;
use qa_core::cvar::CvarRegistry;
use qa_core::identity::{ActorId, OwnedActor, ProviderId, SessionId};
use qa_core::math::{Axis, Bounds, Vec3};
use qa_core::numeric::NumericProfile;
use qa_core::time::{ClockProfile, SourceTime};
use qa_world::session::{MissionGate, ResourceScope, SessionResource};
use qa_world::WorldError;
use thiserror::Error;

/// Largest exactly representable integer (`Number.MAX_SAFE_INTEGER`).
pub const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// Contract validation failure (donor `RangeError`/`Error` throws).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ContractError {
    /// Invalid identity component or identifier.
    #[error("{0}")]
    Invalid(String),
}

fn invalid(message: String) -> ContractError {
    ContractError::Invalid(message)
}

/// Percent-encode per JavaScript `encodeURIComponent`.
///
/// Mirrors `encode_uri_component` in `qa-app` persistence; the unescaped
/// set is ASCII alphanumerics plus `-_.!~*'()`.
#[must_use]
pub fn encode_uri_component(text: &str) -> String {
    let mut out = String::new();
    for byte in text.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')')
        {
            out.push(*byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

macro_rules! id_type {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(pub String);

        impl $name {
            /// Borrow the identifier text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id_type!(
    /// Content identity (`family:edition:package:revision`).
    ContentId
);
id_type!(
    /// Launch recipe identity (`recipe:namespace:revision`).
    RecipeId
);
id_type!(
    /// Resolved resource identity (`resource:...`).
    ResourceId
);
id_type!(
    /// Mount identity (`mount:namespace:name`).
    MountId
);
id_type!(
    /// Mount plan identity (`mount-plan:namespace:revision`).
    MountPlanId
);
id_type!(
    /// Content digest (`sha256:...`).
    ContentDigest
);
id_type!(
    /// Guest callback identity (`namespace:name`).
    CallbackId
);

/// Item identifier (`namespace:name`).
pub type ItemId = String;
/// Objective identifier (`namespace:name`).
pub type ObjectiveId = String;

/// Game family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GameFamily {
    /// Quake.
    Q1,
    /// Quake II.
    Q2,
    /// Quake III.
    Q3,
}

impl Display for GameFamily {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GameFamily::Q1 => f.write_str("q1"),
            GameFamily::Q2 => f.write_str("q2"),
            GameFamily::Q3 => f.write_str("q3"),
        }
    }
}

/// Structured content identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContentIdentity {
    /// Game family.
    pub family: GameFamily,
    /// Edition component.
    pub edition: String,
    /// Package component.
    pub package: String,
    /// Revision component.
    pub revision: String,
}

fn identity_part(value: &str) -> Result<&str, ContractError> {
    let mut chars = value.chars();
    let first = chars.next();
    let valid = matches!(first, Some(c) if c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'));
    if valid {
        Ok(value)
    } else {
        Err(invalid(format!("Invalid content identity component: {value}")))
    }
}

fn identity_component(value: &str) -> bool {
    identity_part(value).is_ok()
}

/// Build a [`ContentId`] from its structured identity.
pub fn create_content_id(identity: &ContentIdentity) -> Result<ContentId, ContractError> {
    Ok(ContentId(format!(
        "{}:{}:{}:{}",
        identity.family,
        identity_part(&identity.edition)?,
        identity_part(&identity.package)?,
        identity_part(&identity.revision)?
    )))
}

/// Check the `family:edition:package:revision` shape.
#[must_use]
pub fn is_content_id(value: &str) -> bool {
    let mut parts = value.split(':');
    match (parts.next(), parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(family), Some(a), Some(b), Some(c), None) => {
            matches!(family, "q1" | "q2" | "q3")
                && identity_component(a)
                && identity_component(b)
                && identity_component(c)
        }
        _ => false,
    }
}

/// Build a [`RecipeId`].
pub fn create_recipe_id(namespace: &str, revision: &str) -> Result<RecipeId, ContractError> {
    Ok(RecipeId(format!(
        "recipe:{}:{}",
        identity_part(namespace)?,
        identity_part(revision)?
    )))
}

/// Build a [`MountId`].
pub fn create_mount_id(namespace: &str, name: &str) -> Result<MountId, ContractError> {
    Ok(MountId(format!(
        "mount:{}:{}",
        identity_part(namespace)?,
        identity_part(name)?
    )))
}

/// Build a [`MountPlanId`].
pub fn create_mount_plan_id(namespace: &str, revision: &str) -> Result<MountPlanId, ContractError> {
    Ok(MountPlanId(format!(
        "mount-plan:{}:{}",
        identity_part(namespace)?,
        identity_part(revision)?
    )))
}

/// Build a [`ContentDigest`] from 64 hexadecimal digits.
pub fn create_content_digest(hex: &str) -> Result<ContentDigest, ContractError> {
    if hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(ContentDigest(format!("sha256:{}", hex.to_lowercase())))
    } else {
        Err(invalid(
            "Expected a SHA-256 digest with 64 hexadecimal digits".to_string(),
        ))
    }
}

/// Check the lowercase `sha256:...` shape.
#[must_use]
pub fn is_content_digest(value: &str) -> bool {
    value.len() == 7 + 64
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
}

/// A new generation identifies replacement bytes or a remounted root.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MountIdentity {
    /// Mount identity.
    pub id: MountId,
    /// Mounted content.
    pub content: ContentId,
    /// Mount generation.
    pub generation: u64,
}

/// Build a [`MountIdentity`].
pub fn create_mount_identity(id: MountId, content: ContentId, generation: u64) -> Result<MountIdentity, ContractError> {
    if generation > MAX_SAFE_INTEGER {
        return Err(invalid(
            "Mount generation must be a nonnegative safe integer".to_string(),
        ));
    }
    Ok(MountIdentity {
        id,
        content,
        generation,
    })
}

/// Archive container format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArchiveFormat {
    /// Quake PAK.
    Pak,
    /// Quake III PK3.
    Pk3,
    /// Kingpin KPF.
    Kpf,
    /// Plain ZIP.
    Zip,
}

/// Archive-backed mount.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArchiveMount {
    /// Mount identity.
    pub identity: MountIdentity,
    /// Container format.
    pub format: ArchiveFormat,
    /// Archive file path.
    pub archive_path: String,
    /// Archive digest.
    pub archive_digest: ContentDigest,
}

/// Directory-backed mount.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LooseMount {
    /// Mount identity.
    pub identity: MountIdentity,
    /// Root directory path.
    pub root_path: String,
}

/// Mounted content source.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ContentMount {
    /// Archive mount.
    Archive(ArchiveMount),
    /// Loose mount.
    Loose(LooseMount),
}

impl ContentMount {
    /// Borrow the mount identity.
    #[must_use]
    pub fn identity(&self) -> &MountIdentity {
        match self {
            ContentMount::Archive(mount) => &mount.identity,
            ContentMount::Loose(mount) => &mount.identity,
        }
    }
}

/// Provenance of resolved resource bytes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ResourceProvenance {
    /// Archive member bytes.
    Archive {
        /// Owning mount.
        mount: ArchiveMount,
        /// Member path.
        member_path: String,
        /// Member index.
        member_index: u64,
    },
    /// Loose file bytes.
    Loose {
        /// Owning mount.
        mount: LooseMount,
        /// Member path.
        member_path: String,
    },
}

/// Prefix mount order; highest priority first.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PrefixMountOrder {
    /// Path prefix.
    pub prefix: String,
    /// Mounts, highest priority first.
    pub mounts: Vec<MountId>,
}

/// Resolved mount plan.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResolvedMountPlan {
    /// Plan identity.
    pub id: MountPlanId,
    /// Mounts.
    pub mounts: Vec<ContentMount>,
    /// Default order, highest priority first.
    pub default_order: Vec<MountId>,
    /// Prefix orders; first matching prefix wins.
    pub prefix_orders: Vec<PrefixMountOrder>,
}

/// How a resource won precedence.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ResourceResolution {
    /// Won through the default order.
    DefaultOrder {
        /// Plan identity.
        plan: MountPlanId,
        /// Rank.
        rank: u64,
    },
    /// Won through a prefix order.
    PrefixOrder {
        /// Plan identity.
        plan: MountPlanId,
        /// Prefix.
        prefix: String,
        /// Rank.
        rank: u64,
    },
    /// Won through a link.
    Link {
        /// Plan identity.
        plan: MountPlanId,
        /// Source prefix.
        source_prefix: String,
        /// Target path.
        target_path: String,
    },
}

/// Records the selected byte identity and mount generation across remounts.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResolvedResourceReference {
    /// Resource identity.
    pub id: ResourceId,
    /// Requested path.
    pub requested_path: String,
    /// Byte provenance.
    pub provenance: ResourceProvenance,
    /// Byte digest.
    pub digest: ContentDigest,
    /// Byte length.
    pub byte_length: u64,
    /// Precedence decision.
    pub resolution: ResourceResolution,
}

fn resource_path(value: &str) -> Result<&str, ContractError> {
    let valid = !value.is_empty()
        && !value.contains('\0')
        && !value.starts_with('/')
        && !value.contains('\\')
        && !value
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..");
    if valid {
        Ok(value)
    } else {
        Err(invalid(format!("Expected a relative resource path: {value}")))
    }
}

fn safe_integer(value: u64, label: &str) -> Result<u64, ContractError> {
    if value <= MAX_SAFE_INTEGER {
        Ok(value)
    } else {
        Err(invalid(format!("{label} must be a nonnegative safe integer")))
    }
}

fn resolution_key(resolution: &ResourceResolution) -> String {
    match resolution {
        ResourceResolution::DefaultOrder { plan, rank } => format!("{plan}:default:{rank}"),
        ResourceResolution::PrefixOrder { plan, prefix, rank } => {
            format!("{plan}:prefix:{}:{rank}", encode_uri_component(prefix))
        }
        ResourceResolution::Link {
            plan,
            source_prefix,
            target_path,
        } => format!(
            "{plan}:link:{}:{}",
            encode_uri_component(source_prefix),
            encode_uri_component(target_path)
        ),
    }
}

/// Resource fields feeding [`create_resource_id`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UnresolvedResourceReference {
    /// Requested path.
    pub requested_path: String,
    /// Byte provenance.
    pub provenance: ResourceProvenance,
    /// Byte digest.
    pub digest: ContentDigest,
    /// Byte length.
    pub byte_length: u64,
    /// Precedence decision.
    pub resolution: ResourceResolution,
}

/// Identity includes the chosen archive/root, member, bytes and precedence decision.
pub fn create_resource_id(resource: &UnresolvedResourceReference) -> Result<ResourceId, ContractError> {
    let identity = match &resource.provenance {
        ResourceProvenance::Archive { mount, .. } => &mount.identity,
        ResourceProvenance::Loose { mount, .. } => &mount.identity,
    };
    let member_path = match &resource.provenance {
        ResourceProvenance::Archive { member_path, .. } => member_path,
        ResourceProvenance::Loose { member_path, .. } => member_path,
    };
    resource_path(&resource.requested_path)?;
    resource_path(member_path)?;
    safe_integer(resource.byte_length, "Resource byte length")?;
    match &resource.resolution {
        ResourceResolution::DefaultOrder { rank, .. } | ResourceResolution::PrefixOrder { rank, .. } => {
            safe_integer(*rank, "Resource precedence rank")?;
        }
        ResourceResolution::Link { .. } => {}
    }
    if let ResourceProvenance::Archive { member_index, .. } = &resource.provenance {
        safe_integer(*member_index, "Archive member index")?;
    }
    let source = match &resource.provenance {
        ResourceProvenance::Archive {
            mount, member_index, ..
        } => format!(
            "{}:{}:{member_index}",
            encode_uri_component(&mount.archive_path),
            mount.archive_digest
        ),
        ResourceProvenance::Loose { mount, .. } => encode_uri_component(&mount.root_path),
    };
    Ok(ResourceId(format!(
        "resource:{}:{}:{}:{source}:{}:{}:{}",
        identity.content,
        identity.id,
        identity.generation,
        encode_uri_component(member_path),
        resource.digest,
        resolution_key(&resource.resolution)
    )))
}

/// Provider operating on content.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProviderReference {
    /// Provider identity.
    pub provider: ProviderId,
    /// Content identity.
    pub content: ContentId,
}

/// Unresolved content path request.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResourceRequest {
    /// Content identity.
    pub content: ContentId,
    /// Resource path.
    pub path: String,
}

/// Keep the preset decision or use an explicit selection.
#[derive(Debug, Clone, PartialEq)]
pub enum LaunchSelection<T> {
    /// Use the preset decision.
    Preset,
    /// Use the explicit selection.
    Selected(T),
}

/// Map geometry plus entity provider.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MapSelection {
    /// Map geometry.
    pub geometry: ResourceRequest,
    /// Entity provider.
    pub entities: ProviderReference,
}

/// Campaign selection; gamecode is independently chosen.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CampaignSelection {
    /// No campaign.
    None,
    /// Campaign mission plus gamecode.
    Campaign {
        /// Mission provider.
        mission: ProviderReference,
        /// Gamecode provider.
        gamecode: ProviderReference,
    },
}

/// Character definition plus appearance.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CharacterSelection {
    /// Character definition.
    pub definition: ProviderReference,
    /// Character appearance.
    pub appearance: ProviderReference,
}

/// Monster definition reference.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MonsterDefinitionReference {
    /// Defining provider.
    pub source: ProviderReference,
    /// Entity classname.
    pub classname: String,
}

/// Monster selection target.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MonsterSelectionTarget {
    /// Explicit definition.
    Defined(MonsterDefinitionReference),
    /// Map-defined monster.
    MapDefined,
}

/// Enemy selection.
#[derive(Debug, Clone, PartialEq)]
pub enum EnemySelection {
    /// Keep map-defined enemies.
    MapDefined,
    /// Replace enemies.
    Replace {
        /// Default replacement.
        default: MonsterSelectionTarget,
        /// Per-classname replacements.
        by_classname: HashMap<String, MonsterSelectionTarget>,
    },
}

/// Grapple mechanic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GrappleMechanic {
    /// Quake Threewave hook.
    Q1Threewave,
    /// Quake II CTF grapple.
    Q2Ctf,
    /// Quake II Lithium CTF grapple.
    Q2Lmctf,
    /// Quake III QVM grapple.
    Q3Qvm,
}

/// Source edition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceEdition {
    /// Classic release.
    Classic,
    /// Rerelease.
    Rerelease,
}

/// Grapple equipment selection.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum GrappleSelection {
    /// Grapple disabled.
    Disabled,
    /// Grapple enabled.
    Enabled {
        /// Defining provider.
        source: ProviderReference,
        /// Input binding.
        binding: GrappleBinding,
        /// Mechanic plus edition.
        mechanic: GrappleMechanicDetail,
    },
}

/// Grapple input binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GrappleBinding {
    /// Weapon slot binding.
    Slot,
    /// Offhand binding.
    Offhand,
}

/// Grapple mechanic plus edition.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum GrappleMechanicDetail {
    /// Quake Threewave hook.
    Q1Threewave {
        /// Edition.
        edition: SourceEdition,
    },
    /// Quake II CTF grapple.
    Q2Ctf {
        /// Edition.
        edition: SourceEdition,
    },
    /// Quake II Lithium CTF grapple, classic only.
    Q2Lmctf,
    /// Quake III QVM grapple, classic only.
    Q3Qvm {
        /// Hook component declaration (donor `qvm-grapple.ts`).
        profile: QvmGrappleDefinition,
    },
}

/// Hand grenade equipment selection.
#[derive(Debug, Clone, PartialEq)]
pub enum HandGrenadeSelection {
    /// Hand grenades disabled.
    Disabled,
    /// Hand grenades enabled.
    Enabled {
        /// Defining provider.
        source: ProviderReference,
        /// Edition.
        edition: SourceEdition,
        /// Initial ammo.
        initial_ammo: f64,
        /// Capacity.
        capacity: f64,
    },
}

/// Equipment selection; never changes maps, gamecode or the primary arsenal.
#[derive(Debug, Clone, PartialEq)]
pub struct EquipmentSelection {
    /// Grapple selection.
    pub grapple: GrappleSelection,
    /// Hand grenade selection.
    pub hand_grenades: HandGrenadeSelection,
}

/// Doppler selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DopplerSelection {
    /// Source doppler.
    Source,
    /// Doppler disabled.
    Disabled,
}

/// Environment selection.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EnvironmentSelection {
    /// Source audio content.
    AudioContent,
    /// Environment disabled.
    Disabled,
    /// Selected resource.
    Selected {
        /// Environment resource.
        resource: ResourceRequest,
    },
}

/// Presentation selection.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PresentationSelection {
    /// Doppler selection.
    pub doppler: DopplerSelection,
    /// Environment selection.
    pub environment: EnvironmentSelection,
    /// Presentation assets.
    pub assets: ContentId,
    /// HUD provider.
    pub hud: ProviderReference,
    /// Effects provider.
    pub effects: ProviderReference,
    /// Audio provider.
    pub audio: ProviderReference,
}

// Boundary execution identities (donor `execution.ts`).

/// QuakeC API identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QuakeCApiIdentity {
    /// NetQuake progs v6, system CRC 5927.
    Netquake,
    /// QuakeWorld progs v6, system CRC 54730.
    Quakeworld,
}

/// Quake II game API identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2GameApiIdentity {
    /// Classic game API version 3.
    Classic,
    /// Rerelease game API version 2023.
    Rerelease,
}

/// Quake II rerelease client-game API identity (version 2022).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Q2CgameApiIdentity;

/// Quake III API identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q3ApiIdentity {
    /// Server game, version 7 or 8.
    Qagame(u8),
    /// Client game, version 3 or 4.
    Cgame(u8),
    /// UI, version 4 or 6.
    Ui(u8),
}

/// Module identity; a replacement identifies the artifact it replaces.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModuleIdentity {
    /// Module provider.
    pub id: ProviderId,
    /// Artifact path.
    pub artifact_path: String,
    /// Artifact digest.
    pub digest: ContentDigest,
    /// Revision.
    pub revision: String,
}

/// Native ABI (donor `execution.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeAbi {
    /// Windows i386 PE32 cdecl.
    WindowsI386,
    /// Windows x86-64 PE32+ Microsoft x64 call.
    WindowsX86_64,
    /// Linux i386 ELF32 System V.
    LinuxI386,
    /// Linux x86-64 ELF64 System V.
    LinuxX86_64,
}

/// Native call ABI; Win32 imports may use a distinct convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeCallAbi {
    /// Plain native ABI.
    Native(NativeAbi),
    /// Windows i386 with stdcall/thiscall/fastcall.
    WindowsI386Alt(WindowsI386Call),
}

/// Windows i386 alternate calling convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowsI386Call {
    /// stdcall.
    Stdcall,
    /// thiscall.
    Thiscall,
    /// fastcall.
    Fastcall,
}

/// QVM ABI profile (donor `execution.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QvmAbiProfile {
    /// Modern Q3 ABI.
    Modern,
    /// Quake III 1.16n base ABI.
    Legacy116n,
}

/// Module role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModuleRole {
    /// Server game.
    ServerGame,
    /// Client game.
    ClientGame,
    /// UI.
    Ui,
}

/// Native module profile.
pub type NativeModuleProfile = NativeAbi;

/// Execution module; execution form chooses no engine behavior.
#[derive(Debug, Clone, PartialEq)]
pub enum ExecutionModule<Artifact> {
    /// TypeScript implementation.
    Typescript {
        /// Owning provider.
        owner: ProviderReference,
        /// Implementation provider.
        implementation: ProviderId,
        /// Module role.
        role: ModuleRole,
        /// Source module API.
        api: SourceModuleApi,
    },
    /// QuakeC artifact, server game only.
    Quakec {
        /// Owning provider.
        owner: ProviderReference,
        /// Artifact.
        artifact: Artifact,
        /// API identity.
        api: QuakeCApiIdentity,
    },
    /// QVM artifact.
    Qvm {
        /// Owning provider.
        owner: ProviderReference,
        /// Artifact.
        artifact: Artifact,
        /// Module role.
        role: ModuleRole,
        /// API identity.
        api: Q3ApiIdentity,
    },
    /// Native artifact.
    Native {
        /// Owning provider.
        owner: ProviderReference,
        /// Artifact.
        artifact: Artifact,
        /// ABI profile.
        profile: NativeModuleProfile,
        /// Module role.
        role: ModuleRole,
        /// Native module API.
        api: NativeModuleApi,
    },
}

/// Source module API (QuakeC server game plus native roles).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceModuleApi {
    /// QuakeC server game.
    Quakec(QuakeCApiIdentity),
    /// Quake II classic game.
    Q2ClassicGame,
    /// Quake II rerelease game.
    Q2RereleaseGame,
    /// Quake II rerelease client game.
    Q2RereleaseCgame,
    /// Quake III server game.
    Q3Qagame(u8),
    /// Quake III client game.
    Q3Cgame(u8),
    /// Quake III UI.
    Q3Ui(u8),
}

/// Native module API (Q2 plus Q3 roles).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeModuleApi {
    /// Quake II classic game.
    Q2ClassicGame,
    /// Quake II rerelease game.
    Q2RereleaseGame,
    /// Quake II rerelease client game.
    Q2RereleaseCgame,
    /// Quake III server game.
    Q3Qagame(u8),
    /// Quake III client game.
    Q3Cgame(u8),
    /// Quake III UI.
    Q3Ui(u8),
}

/// Unresolved execution selection.
pub type ExecutionSelection = ExecutionModule<ResourceRequest>;
/// Resolved execution module.
pub type ResolvedExecutionModule = ExecutionModule<ResolvedResourceReference>;

/// Launch choice; only explicit selections replace preset decisions.
#[derive(Debug, Clone, PartialEq)]
pub struct LaunchChoice {
    /// Preset recipe.
    pub preset: RecipeId,
    /// Map selection.
    pub map: LaunchSelection<MapSelection>,
    /// Campaign selection.
    pub campaign: LaunchSelection<CampaignSelection>,
    /// Movement provider.
    pub movement: LaunchSelection<ProviderReference>,
    /// Character selection.
    pub character: LaunchSelection<CharacterSelection>,
    /// Weapon providers.
    pub weapons: LaunchSelection<Vec<ProviderReference>>,
    /// Equipment selection.
    pub equipment: LaunchSelection<EquipmentSelection>,
    /// Enemy selection.
    pub enemies: LaunchSelection<EnemySelection>,
    /// Presentation selection.
    pub presentation: LaunchSelection<PresentationSelection>,
    /// Engine behavior provider.
    pub engine_behavior: LaunchSelection<ProviderReference>,
    /// Combat provider.
    pub combat: LaunchSelection<ProviderReference>,
    /// Inventory provider.
    pub inventory: LaunchSelection<ProviderReference>,
    /// Match provider.
    pub r#match: LaunchSelection<ProviderReference>,
    /// Transition provider.
    pub transition: LaunchSelection<ProviderReference>,
    /// Execution modules.
    pub execution: LaunchSelection<Vec<ExecutionSelection>>,
}

/// Resolved map.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResolvedMap {
    /// Selected map product.
    pub geometry_content: ContentId,
    /// Map geometry.
    pub geometry: ResolvedResourceReference,
    /// Entity provider.
    pub entities: ProviderReference,
}

/// Provider timing.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderTiming {
    /// Provider identity.
    pub provider: ProviderId,
    /// Clock profile.
    pub clock: ClockProfile,
    /// Numeric profile.
    pub numeric: NumericProfile,
}

/// Resolved weapon behavior selection (donors `weapon-behavior.ts`,
/// `native-weapon-behavior.ts`, `execution.ts`).
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedWeaponBehaviorSelection {
    /// Behavior component.
    pub component: Option<WeaponBehaviorComponent>,
    /// Defining provider.
    pub source: ProviderReference,
    /// Behavior artifact.
    pub artifact: ResolvedResourceReference,
    /// Behavior definition.
    pub definition: WeaponBehaviorDefinition,
}

/// Weapon behavior component.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum WeaponBehaviorComponent {
    /// QVM component.
    Qvm {
        /// ABI profile.
        abi_profile: QvmAbiProfile,
        /// Behavior layout.
        layout: QvmWeaponBehaviorLayout,
    },
    /// Rerelease native component.
    RereleaseNative {
        /// Behavior declaration.
        declaration: NativeWeaponBehaviorDeclaration,
    },
}

/// Executable recipe, resolved before session construction.
#[derive(Debug, Clone, PartialEq)]
pub struct ExecutableRecipe {
    /// Weapon behaviors.
    pub weapon_behaviors: Vec<ResolvedWeaponBehaviorSelection>,
    /// Gameplay mods.
    pub mods: Vec<ResolvedGameplayMod>,
    /// Schema version (3).
    pub schema_version: u32,
    /// Recipe identity.
    pub id: RecipeId,
    /// Preset recipe.
    pub preset: RecipeId,
    /// Resolved map.
    pub map: ResolvedMap,
    /// Campaign selection.
    pub campaign: CampaignSelection,
    /// Movement provider.
    pub movement: ProviderReference,
    /// Character selection.
    pub character: CharacterSelection,
    /// Weapon providers.
    pub weapons: Vec<ProviderReference>,
    /// Equipment selection.
    pub equipment: EquipmentSelection,
    /// Enemy selection.
    pub enemies: EnemySelection,
    /// Presentation selection.
    pub presentation: PresentationSelection,
    /// Engine behavior provider.
    pub engine_behavior: ProviderReference,
    /// Combat provider.
    pub combat: ProviderReference,
    /// Inventory provider.
    pub inventory: ProviderReference,
    /// Match provider.
    pub r#match: ProviderReference,
    /// Transition provider.
    pub transition: ProviderReference,
    /// Execution modules.
    pub execution: Vec<ResolvedExecutionModule>,
    /// Mount plan.
    pub mounts: ResolvedMountPlan,
    /// Resolved resources.
    pub resources: Vec<ResolvedResourceReference>,
    /// Provider timing.
    pub timing: Vec<ProviderTiming>,
    /// Frame ordering (donor `time.ts`).
    pub ordering: FrameOrdering,
}

/// Frame ordering (donor `time.ts`).
#[derive(Debug, Clone, PartialEq)]
pub enum FrameOrdering {
    /// Native traversal in source slot order.
    Native {
        /// Clock profile.
        clock: ClockProfile,
    },
    /// Mixed providers.
    Mixed {
        /// Providers.
        providers: Vec<ProviderId>,
    },
}

// Boundary gameplay types (donor `gameplay.ts`).

/// Inventory count policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InventoryCountPolicy {
    /// Nonnegative stack.
    Stack,
    /// Signed source counter.
    SourceCounter(SourceCounterArithmetic),
}

/// Source counter arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceCounterArithmetic {
    /// 32-bit binary float.
    Binary32,
    /// 64-bit binary float.
    Binary64,
    /// 32-bit integer.
    Int32,
}

/// Inventory entry.
#[derive(Debug, Clone, PartialEq)]
pub struct InventoryEntry {
    /// Item identifier.
    pub item: ItemId,
    /// Current count.
    pub count: f64,
    /// Capacity.
    pub capacity: f64,
    /// Count policy; absence on a new entry selects a nonnegative stack.
    pub count_policy: Option<InventoryCountPolicy>,
}

/// Protection channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProtectionChannel {
    /// Regular armor.
    Regular,
    /// Powered protection.
    Powered,
}

/// Regular armor state.
#[derive(Debug, Clone, PartialEq)]
pub enum RegularArmorState {
    /// No armor.
    None,
    /// Quake armor.
    Q1 {
        /// Armor points.
        points: f64,
        /// Absorption.
        absorption: f64,
        /// Armor item.
        item: ItemId,
    },
    /// Quake II armor.
    Q2 {
        /// Armor points.
        points: f64,
        /// Normal protection.
        normal_protection: f64,
        /// Energy protection.
        energy_protection: f64,
        /// Armor item.
        item: ItemId,
    },
    /// Quake III armor.
    Q3 {
        /// Armor points.
        points: f64,
        /// Protection.
        protection: f64,
    },
    /// Source armor.
    Source {
        /// Armor points.
        points: f64,
        /// Armor item.
        item: Option<ItemId>,
    },
}

/// Powered protection state.
#[derive(Debug, Clone, PartialEq)]
pub enum PoweredProtectionState {
    /// No protection.
    None,
    /// Screen protection.
    Screen {
        /// Cells.
        cells: f64,
    },
    /// Shield protection.
    Shield {
        /// Cells.
        cells: f64,
    },
}

/// Armor state.
#[derive(Debug, Clone, PartialEq)]
pub struct ArmorState {
    /// Regular armor.
    pub regular: RegularArmorState,
    /// Powered protection.
    pub powered: PoweredProtectionState,
}

/// Committed protection store across owned channels.
#[derive(Debug, Clone, PartialEq)]
pub struct ProtectionStore {
    /// Regular channel change.
    pub regular: Option<ProtectionChange<RegularArmorState>>,
    /// Powered channel change.
    pub powered: Option<ProtectionChange<PoweredProtectionState>>,
}

/// Before/after protection change.
#[derive(Debug, Clone, PartialEq)]
pub struct ProtectionChange<State> {
    /// State before.
    pub before: State,
    /// State after.
    pub after: State,
}

/// Reports one committed store across owned channels; never repeats a write.
pub trait ProtectionObserver {
    /// Record a committed store.
    fn stored(&self, change: &ProtectionStore);
}

/// Model transform (donor `scene.ts`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelTransform {
    /// Origin.
    pub origin: Vec3,
    /// Axis.
    pub axis: Axis,
    /// Scale.
    pub scale: Vec3,
}

/// HUD icon (donor `ui.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WeaponHudIcon {
    /// WAD picture resource.
    WadPicture {
        /// Picture resource.
        resource: ResourceRequest,
        /// Lump name.
        lump: String,
    },
    /// Image resource.
    Image {
        /// Image resource.
        resource: ResourceRequest,
    },
    /// Shader.
    Shader {
        /// Content identity.
        content: ContentId,
        /// Shader name.
        name: String,
    },
}

// `source-items.ts`.

/// Declared original cadence context for a provider's equipment.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceEquipmentContext {
    /// Provider identity.
    pub provider: ProviderId,
    /// Equipment item.
    pub item: Option<ItemId>,
}

/// Resolve the equipment item for exactly one declared context.
pub fn source_equipment_item(
    contexts: &[SourceEquipmentContext],
    provider: &ProviderId,
) -> Result<Option<ItemId>, ContractError> {
    let matches: Vec<&SourceEquipmentContext> = contexts
        .iter()
        .filter(|context| context.provider == *provider)
        .collect();
    let Some(context) = matches.first() else {
        return Err(invalid(format!(
            "Equipment {provider:?} has no declared original cadence context"
        )));
    };
    if matches.len() != 1 {
        return Err(invalid(format!(
            "Equipment {provider:?} has duplicate original cadence contexts"
        )));
    }
    Ok(context.item.clone())
}

/// Source item action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceItemAction {
    /// Use the item.
    Use,
    /// Drop the item.
    Drop,
}

/// Source item action calls.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceItemActionCalls<Call> {
    /// Use call.
    pub use_call: Option<Call>,
    /// Drop call.
    pub drop_call: Option<Call>,
}

/// Names of the declared action calls.
#[must_use]
pub fn source_item_action_names<Call>(calls: &SourceItemActionCalls<Call>) -> Vec<SourceItemAction> {
    let mut names = Vec::new();
    if calls.use_call.is_some() {
        names.push(SourceItemAction::Use);
    }
    if calls.drop_call.is_some() {
        names.push(SourceItemAction::Drop);
    }
    names
}

/// Item lookup key for [`source_item_named`].
pub trait SourceItemKey {
    /// Canonical item identifier.
    fn item_id(&self) -> &str;
    /// Display label.
    fn label(&self) -> &str;
}

/// Named item lookup result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceItemMatch<'a, Item> {
    /// Single match.
    Match {
        /// Matched item.
        item: &'a Item,
        /// Exact canonical ID match.
        exact: bool,
    },
    /// Ambiguous label matches.
    Ambiguous {
        /// Matching items.
        items: Vec<&'a Item>,
    },
}

fn normalized_item_text(text: &str) -> String {
    text.to_lowercase().chars().filter(|c| *c != ' ').collect()
}

/// Find an item by canonical ID or display label.
///
/// Exact canonical IDs take precedence over display names shared by
/// composed sources.
pub fn source_item_named<'a, Item: SourceItemKey>(
    items: &'a [Item],
    text: &str,
    original: Option<&ItemId>,
) -> Option<SourceItemMatch<'a, Item>> {
    let requested = normalized_item_text(text);
    if let Some(exact) = items.iter().find(|item| item.item_id().to_lowercase() == requested) {
        return Some(SourceItemMatch::Match {
            item: exact,
            exact: true,
        });
    }
    if let Some(original) = original {
        if let Some(primary) = items.iter().find(|item| item.item_id() == original.as_str()) {
            return Some(SourceItemMatch::Match {
                item: primary,
                exact: false,
            });
        }
    }
    let matches: Vec<&Item> = items
        .iter()
        .filter(|item| normalized_item_text(item.label()) == requested)
        .collect();
    if matches.len() > 1 {
        Some(SourceItemMatch::Ambiguous { items: matches })
    } else {
        matches
            .first()
            .copied()
            .map(|item| SourceItemMatch::Match { item, exact: false })
    }
}

/// Source item icon declaration.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SourceItemIconDeclaration {
    /// Image resource.
    Image {
        /// Resource path.
        path: String,
    },
    /// WAD picture resource.
    WadPicture {
        /// Resource path.
        path: String,
        /// Lump name.
        lump: String,
    },
    /// Shader.
    Shader {
        /// Shader name.
        name: String,
    },
}

/// Source weapon item detail.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceWeaponItem {
    /// Ammo item.
    pub ammo: Option<ItemId>,
    /// Held weapon declaration.
    pub held: Option<HeldWeaponDeclaration>,
}

/// Source item definition.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceItemDefinition {
    /// Item identifier.
    pub item: ItemId,
    /// Display label.
    pub label: String,
    /// Defining provider.
    pub source: ProviderReference,
    /// HUD icon.
    pub icon: Option<WeaponHudIcon>,
    /// Declared actions.
    pub actions: Vec<SourceItemAction>,
    /// Item kind.
    pub kind: SourceItemKind,
}

/// Source item kind.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum SourceItemKind {
    /// Counter item.
    Counter,
    /// Weapon item.
    Weapon(SourceWeaponItem),
}

impl SourceItemKey for SourceItemDefinition {
    fn item_id(&self) -> &str {
        &self.item
    }

    fn label(&self) -> &str {
        &self.label
    }
}

/// Source item admission.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceItemAdmission {
    /// Item definition.
    pub definition: SourceItemDefinition,
    /// Admission mode.
    pub admission: SourceItemAdmissionMode,
}

/// Source item admission mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceItemAdmissionMode {
    /// Add the definition.
    Add,
    /// Replace the primary definition.
    ReplacePrimary,
}

/// Committed inventory store.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceItemStore {
    /// Entry before.
    pub before: InventoryEntry,
    /// Entry after.
    pub after: InventoryEntry,
}

/// Source item lease.
pub trait SourceItemLease {
    /// Whether the lease is current.
    fn current(&self) -> bool;
    /// Record committed stores.
    fn stored(&self, changes: &[SourceItemStore]);
    /// Close the lease.
    fn close(&self);
}

/// Committed inventory source is no longer current.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceItemRetired {
    /// Actor holding the lease.
    pub actor: OwnedActor,
    /// Lease owner.
    pub owner: ProviderId,
}

impl Display for SourceItemRetired {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Committed inventory source is no longer current")
    }
}

impl std::error::Error for SourceItemRetired {}

/// Provider weapon reference.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WeaponReference {
    /// Provider identity.
    pub provider: ProviderId,
    /// Weapon item.
    pub item: ItemId,
}

/// Source weapon selection request.
pub trait SourceWeaponRequest {
    /// Request identity.
    fn id(&self) -> u64;
    /// Request status.
    fn status(&self) -> SourceWeaponRequestStatus;
    /// Cancel the request.
    fn cancel(&self);
}

/// Source weapon request status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceWeaponRequestStatus {
    /// Pending.
    Pending,
    /// Accepted.
    Accepted,
    /// Refused.
    Refused,
}

/// Shared source weapon handoff operations.
pub trait SourceWeaponHandoffBase {
    /// Provider identity.
    fn provider(&self) -> &ProviderId;
    /// Whether the handoff accepts an item.
    fn accepts(&self, item: &ItemId) -> bool;
    /// Select an item.
    fn select(&self, item: &ItemId) -> bool;
    /// Holster the weapon.
    fn holster(&self);
    /// Whether the weapon is holstered.
    fn is_holstered(&self) -> bool;
}

/// Immediate source weapon handoff.
pub trait ImmediateWeaponHandoff: SourceWeaponHandoffBase {
    /// Resume an item.
    fn resume(&self, item: Option<&ItemId>) -> bool;
}

/// Source-input weapon handoff.
pub trait SourceInputWeaponHandoff: SourceWeaponHandoffBase {
    /// Resume an item through a request.
    fn resume(&self, item: Option<&ItemId>) -> Box<dyn SourceWeaponRequest>;
    /// Restore a request.
    fn restore_request(&self, id: u64, item: Option<&ItemId>) -> Box<dyn SourceWeaponRequest>;
}

/// Source weapon handoff.
pub enum SourceWeaponHandoff {
    /// Immediate handoff.
    Immediate(Box<dyn ImmediateWeaponHandoff>),
    /// Source-input handoff.
    SourceInput(Box<dyn SourceInputWeaponHandoff>),
}

/// Presented weapon model.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum SourceWeaponModel {
    /// Resolved resource model.
    Resolved {
        /// Model resource.
        resource: ResolvedResourceReference,
        /// Frame.
        frame: f64,
    },
    /// Source path model.
    SourcePath {
        /// Model path.
        path: String,
        /// Frame.
        frame: f64,
    },
}

/// Source weapon presentation.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceWeaponPresentation {
    /// Defining provider.
    pub source: ProviderReference,
    /// Active item.
    pub active: Option<ItemId>,
    /// Pending item.
    pub pending: Option<ItemId>,
    /// Presented model.
    pub model: Option<SourceWeaponModel>,
    /// Presented items.
    pub items: Vec<SourceItemDefinition>,
}

/// Source weapon binding.
pub trait SourceWeaponBinding {
    /// Bound handoff.
    fn handoff(&self) -> &SourceWeaponHandoff;
    /// Whether the binding is current.
    fn current(&self) -> bool;
    /// Read the presentation.
    fn read(&self) -> SourceWeaponPresentation;
}

/// Actor weapon slot services.
pub trait SourceWeaponServices {
    /// Bind an actor's weapon slot; returns the release callback.
    fn bind(&self, actor: &OwnedActor, binding: &dyn SourceWeaponBinding) -> Box<dyn FnOnce()>;
    /// Whether the provider's weapon is selected.
    fn selected(&self, actor: &ActorId, provider: &ProviderId) -> bool;
    /// Whether the provider's weapon is presented.
    fn presented(&self, actor: &ActorId, provider: &ProviderId) -> bool;
    /// Request a weapon.
    fn request(&self, actor: &ActorId, weapon: &WeaponReference) -> bool;
}

// `held-weapon.ts`.

/// Held weapon declaration.
#[derive(Debug, Clone, PartialEq)]
pub enum HeldWeaponDeclaration {
    /// No held weapon.
    None,
    /// Held weapon model.
    Model(HeldWeaponModel),
}

/// Held weapon model.
#[derive(Debug, Clone, PartialEq)]
pub struct HeldWeaponModel {
    /// Model digest.
    pub digest: Option<ContentDigest>,
    /// Model path.
    pub path: String,
    /// Reference frame.
    pub reference_frame: f64,
    /// Grip transform.
    pub grip: ModelTransform,
    /// Fallback path.
    pub fallback: Option<String>,
    /// Model part.
    pub part: Option<HeldWeaponPart>,
}

/// Held weapon model part.
#[derive(Debug, Clone, PartialEq)]
pub struct HeldWeaponPart {
    /// Part digests.
    pub digests: Vec<String>,
    /// Part vertices.
    pub vertices: Vec<f64>,
}

// `model-attachment.ts`.

/// Model attachment definition.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelAttachmentDefinition {
    /// Attachment digest.
    pub digest: ContentDigest,
    /// Grip transform.
    pub grip: ModelTransform,
    /// Attachment target.
    pub target: ModelAttachmentTarget,
}

/// Model attachment target.
#[derive(Debug, Clone, PartialEq)]
pub enum ModelAttachmentTarget {
    /// Joint attachment.
    Joint {
        /// Joint name.
        name: String,
    },
    /// Mesh attachment.
    Mesh {
        /// Reference frame.
        reference_frame: f64,
        /// Attachment vertices.
        vertices: Vec<[f64; 3]>,
    },
}

// `pickups.ts`.

/// Pickup ammo grant.
#[derive(Debug, Clone, PartialEq)]
pub struct PickupAmmoGrant {
    /// Granted item.
    pub item: ItemId,
    /// Granted amount.
    pub amount: f64,
}

/// Pickup weapon selection policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PickupSelection {
    /// Never select.
    Never,
    /// Always select.
    Always,
    /// Select when better.
    Better,
}

/// Ammo weapon selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AmmoWeaponSelection {
    /// Selection mode.
    pub mode: PickupSelection,
    /// Selection timing.
    pub when: AmmoWeaponTiming,
}

/// Ammo weapon selection timing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AmmoWeaponTiming {
    /// Always.
    Always,
    /// When ammo is empty.
    EmptyAmmo,
}

/// Pickup supply offer.
#[derive(Debug, Clone, PartialEq)]
pub enum PickupSupplyOffer {
    /// Ammo offer.
    Ammo(PickupAmmoGrant),
    /// Ammo weapon offer.
    AmmoWeapon(PickupAmmoWeaponGrant),
    /// Weapon offer.
    Weapon(PickupWeaponGrant),
}

/// Ammo weapon grant.
#[derive(Debug, Clone, PartialEq)]
pub struct PickupAmmoWeaponGrant {
    /// Granted item.
    pub item: ItemId,
    /// Granted amount.
    pub amount: f64,
    /// Granted weapon.
    pub weapon: ItemId,
}

/// Weapon grant.
#[derive(Debug, Clone, PartialEq)]
pub struct PickupWeaponGrant {
    /// Granted item.
    pub item: ItemId,
    /// Granted ammo.
    pub ammo: Vec<PickupAmmoGrant>,
}

/// Pickup supply availability.
#[derive(Debug, Clone, PartialEq)]
pub enum PickupAvailability {
    /// Ready for pickup.
    Ready {
        /// Eligibility.
        eligible: bool,
    },
    /// Respawning.
    Respawning {
        /// Respawn time in source seconds.
        at_seconds: f64,
    },
    /// Inactive.
    Inactive,
}

/// Pickup supply observation.
#[derive(Debug, Clone, PartialEq)]
pub struct PickupSupplyObservation {
    /// Observing actor.
    pub actor: ActorId,
    /// Supply offer.
    pub offer: PickupSupplyOffer,
    /// Availability.
    pub availability: PickupAvailability,
}

/// Pickup supply preview; supply acceptance only.
#[derive(Debug, Clone, PartialEq)]
pub struct PickupSupplyPreview {
    /// Whether the offer is accepted.
    pub accepted: bool,
    /// Ammo receipts.
    pub ammo: Vec<PickupAmmoReceipt>,
    /// Weapon receipts.
    pub weapons: Vec<PickupAmmoReceipt>,
}

/// Pickup admission; source touch eligibility stays with the pickup owner.
pub trait PickupAdmission {
    /// Whether an item maps into a kind.
    fn maps(&self, kind: PickupMapKind, item: &ItemId) -> bool;
    /// Preview an offer.
    fn preview(&self, actor: &ActorId, offer: &PickupSupplyOffer) -> PickupSupplyPreview;
    /// Whether an actor owns a source weapon.
    fn owns(&self, actor: &ActorId, source_weapon: &ItemId) -> bool;
    /// Grant ammo.
    fn ammo(&self, actor: &OwnedActor, offer: &PickupAmmoGrant, auto_switch: bool) -> bool;
    /// Grant an ammo weapon.
    fn ammo_weapon(&self, actor: &OwnedActor, offer: &PickupAmmoWeaponGrant, selection: AmmoWeaponSelection) -> bool;
    /// Grant a weapon.
    fn weapon(&self, actor: &OwnedActor, offer: &PickupWeaponGrant, selection: PickupSelection) -> bool;
    /// Grant cargo.
    fn cargo(&self, actor: &OwnedActor, cargo: &[PickupCargoEntry], selection: PickupSelection) -> bool;
}

/// Pickup mapping kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PickupMapKind {
    /// Ammo mapping.
    Ammo,
    /// Weapon mapping.
    Weapons,
}

/// Pickup supply profile.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PickupSupplyProfile {
    /// Profile item.
    pub id: ItemId,
    /// Ammo routes.
    pub ammo: Vec<PickupRoute>,
    /// Weapon owner overrides.
    pub weapon_owners: Vec<PickupOwner>,
    /// Ammo owner overrides.
    pub ammo_owners: Vec<PickupOwner>,
    /// Weapon routes.
    pub weapons: Vec<PickupRoute>,
}

/// Pickup route from a source to destinations.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PickupRoute {
    /// Source item.
    pub source: ItemId,
    /// Destination items (at least one).
    pub destinations: Vec<ItemId>,
}

/// Pickup owner override.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PickupOwner {
    /// Item identifier.
    pub item: ItemId,
    /// Source item.
    pub source: ItemId,
}

/// Pickup ammo receipt.
#[derive(Debug, Clone, PartialEq)]
pub struct PickupAmmoReceipt {
    /// Item identifier.
    pub item: ItemId,
    /// Count before.
    pub before: f64,
    /// Count given.
    pub given: f64,
}

/// Source pickup quantity; the pickup owns the signed change and acceptance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourcePickupQuantity {
    /// Signed amount.
    pub amount: f64,
    /// Acceptance.
    pub accepted: bool,
}

// `original-pickups.ts`.

/// Pickup resource binding.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PickupResource {
    /// Protection channel binding.
    Protection {
        /// Channel.
        channel: ProtectionChannel,
    },
    /// Inventory binding.
    Inventory {
        /// Item identifier.
        item: ItemId,
    },
}

/// Pickup write binding.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PickupWrite {
    /// Protection channel write.
    Protection {
        /// Channel.
        channel: ProtectionChannel,
    },
    /// Inventory write.
    Inventory {
        /// Item identifier.
        item: ItemId,
        /// Written fields.
        fields: PickupWriteFields,
    },
}

/// Written inventory fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PickupWriteFields {
    /// Count only.
    Count,
    /// Capacity only.
    Capacity,
    /// Count and capacity.
    CountAndCapacity,
}

/// Pickup count.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PickupCount {
    /// Default count.
    Default,
    /// Override count.
    Override {
        /// Amount.
        amount: f64,
    },
}

/// Pickup cargo entry.
#[derive(Debug, Clone, PartialEq)]
pub struct PickupCargoEntry {
    /// Cargo kind.
    pub kind: PickupCargoKind,
    /// Item identifier.
    pub item: ItemId,
    /// Count.
    pub count: f64,
}

/// Pickup cargo kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PickupCargoKind {
    /// Counter cargo.
    Counter,
    /// Weapon cargo.
    Weapon,
}

/// Original pickup offer.
#[derive(Debug, Clone, PartialEq)]
pub struct OriginalPickupOffer {
    /// Recipient actor.
    pub recipient: ActorId,
    /// Pickup actor.
    pub pickup: ActorId,
    /// Source provider.
    pub source: ProviderId,
    /// Offered item.
    pub item: ItemId,
    /// Default resource binding.
    pub default_resource: Option<PickupResource>,
    /// Pickup count.
    pub count: PickupCount,
    /// Whether the pickup was dropped.
    pub dropped: bool,
    /// Source time.
    pub time: SourceTime,
    /// Cargo entries.
    pub cargo: Vec<PickupCargoEntry>,
    /// Grant mode.
    pub grant: Option<OriginalPickupGrant>,
}

/// Original pickup grant mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OriginalPickupGrant {
    /// Map-coupled grant.
    MapCoupled,
    /// Source effect grant.
    SourceEffect,
}

/// Original pickup decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OriginalPickupDecision {
    /// Accepted.
    Accepted,
    /// Refused.
    Refused,
}

/// Original pickup outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OriginalPickupOutcome {
    /// Accepted.
    Accepted,
    /// Refused.
    Refused,
    /// Stale.
    Stale,
}

/// Original pickup rule; the resource binding owns these rules.
pub trait OriginalPickupRule {
    /// Rule identity.
    fn id(&self) -> &str;
    /// Offered items.
    fn offered(&self) -> &[ItemId];
    /// Pickup writes (at least one).
    fn writes(&self) -> &[PickupWrite];
    /// Take an offer.
    fn take(&self, offer: &OriginalPickupOffer, execution: &dyn OriginalPickupExecution) -> OriginalPickupDecision;
}

/// Original pickup execution.
pub trait OriginalPickupExecution: ProtectionObserver {
    /// Pickup writes (at least one).
    fn writes(&self) -> &[PickupWrite];
    /// Whether the execution is current.
    fn current(&self) -> bool;
}

/// Current original pickup match.
pub struct CurrentOriginalPickup<'a> {
    /// Owning provider.
    pub owner: ProviderId,
    /// Grant operation.
    pub operation: &'a dyn OriginalPickupRule,
    /// Captured operation.
    pub captured: &'a dyn OriginalPickupRule,
    /// Pickup write.
    pub write: PickupWrite,
    /// Currency check.
    pub current: Box<dyn Fn() -> bool + 'a>,
}

/// Original pickup resolution.
pub struct OriginalPickupResolution<'a> {
    /// Current matches.
    pub matches: Vec<CurrentOriginalPickup<'a>>,
    /// Whether the primary grant is blocked.
    pub blocks_primary: bool,
}

/// Original pickup continuation.
pub trait OriginalPickupContinuation {
    /// Map touch eligibility.
    fn eligible(&self) -> bool;
    /// Run the original grant.
    fn original(&self) -> bool;
    /// Complete a settled attempt.
    fn complete(&self, taken: bool);
}

/// Source pickup selection.
pub enum SourcePickupSelection<'a> {
    /// Original grant.
    Original,
    /// Blocked grant.
    Blocked,
    /// Stale grant.
    Stale,
    /// Replacement grant.
    Replacement {
        /// Currency check.
        current: Box<dyn Fn() -> bool + 'a>,
        /// Run the grant.
        grant: Box<dyn Fn() -> OriginalPickupOutcome + 'a>,
    },
}

/// Source pickup lifetime.
pub trait SourcePickupLifetime {
    /// Retire the pickup.
    fn consume_pickup(&self, remove: Box<dyn FnOnce()>);
}

/// Original pickup admission.
pub trait OriginalPickupAdmission {
    /// Run a source grant, keeping the owner and item scope.
    fn run_source<R>(
        &self,
        offer: &OriginalPickupOffer,
        execute: &mut dyn FnMut(SourcePickupSelection<'_>, &dyn SourcePickupLifetime) -> R,
    ) -> R;
    /// Touch a pickup.
    fn touch(
        &self,
        offer: &OriginalPickupOffer,
        continuation: &dyn OriginalPickupContinuation,
    ) -> OriginalPickupOutcome;
}

/// Original pickup operation.
#[derive(Debug, Clone, PartialEq)]
pub enum OriginalPickupOperation<Call> {
    /// Boolean grant.
    BooleanGrant {
        /// Grant call.
        grant: Call,
    },
    /// Gate then grant.
    GateThenGrant {
        /// Gate call.
        gate: Call,
        /// Grant call.
        grant: Call,
        /// Grant acceptance.
        grant_accepts: GrantAcceptance,
    },
}

/// Gate-then-grant acceptance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GrantAcceptance {
    /// Nonzero accepts.
    Nonzero,
    /// Always accepts.
    Always,
}

/// Mod pickup rule.
#[derive(Debug, Clone, PartialEq)]
pub struct ModPickupRule<Call> {
    /// Rule identity.
    pub id: String,
    /// Pickup writes (at least one).
    pub writes: Vec<PickupWrite>,
    /// Offered items.
    pub offered: Vec<ItemId>,
    /// Pickup operation.
    pub operation: OriginalPickupOperation<Call>,
}

// Boundary match/objective types (donor `source-match.ts`; referenced
// shapes only, runtimes stay with the match owner).

/// Source team value.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceTeamValue {
    /// Original value.
    pub value: f64,
    /// Shared team identity.
    pub team: Option<String>,
}

/// Source match field.
#[derive(Debug, Clone, PartialEq)]
pub enum SourceMatchField {
    /// Score binding.
    Score,
    /// Team binding.
    Team {
        /// Team values.
        values: Vec<SourceTeamValue>,
    },
}

/// Source objective value.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceObjectiveValue {
    /// Objective value.
    pub value: f64,
    /// Objective stage.
    pub stage: String,
    /// Completion.
    pub complete: bool,
}

/// Source objective declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceObjectiveDeclaration<Scalar, Reference, Call> {
    /// Objective identity.
    pub id: ObjectiveId,
    /// Objective storage.
    pub storage: Scalar,
    /// Objective values.
    pub values: Vec<SourceObjectiveValue>,
    /// Objective carrier.
    pub carrier: Option<Reference>,
    /// Objective target.
    pub target: Option<Reference>,
    /// Objective role.
    pub role: SourceObjectiveRole<Call>,
}

/// Source objective role.
#[derive(Debug, Clone, PartialEq)]
pub enum SourceObjectiveRole<Call> {
    /// Owned objective.
    Owned {
        /// Campaign gate.
        campaign_gate: bool,
        /// Bot goal.
        bot_goal: bool,
        /// Change call.
        change: Option<Call>,
    },
    /// Borrowed objective.
    Borrowed {
        /// Writable.
        writable: bool,
    },
}

// Boundary client output types (donor `mod-client-outputs.ts`).

/// Mod client movement mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModClientMovementMode {
    /// Normal movement.
    Normal,
    /// Noclip movement.
    Noclip,
    /// Frozen movement.
    Freeze,
}

/// Semantic mod client output.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ModClientOutput {
    /// View offset.
    ViewOffset(Vec3),
    /// Movement mode.
    MovementMode(ModClientMovementMode),
    /// Stance (crouched).
    Stance(bool),
    /// Body shape.
    BodyShape(Bounds),
}

/// Mod client output channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModClientOutputChannel {
    /// View offset channel.
    ViewOffset,
    /// Movement mode channel.
    MovementMode,
    /// Stance channel.
    Stance,
    /// Body shape channel.
    BodyShape,
}

/// Mod client output declaration.
#[derive(Debug, Clone, PartialEq)]
pub enum ModClientOutputDeclaration<Scalar, Vector> {
    /// Body shape declaration.
    BodyShape {
        /// Minimum corner.
        min: Vector,
        /// Maximum corner.
        max: Vector,
    },
    /// View offset field declaration.
    ViewOffsetField {
        /// Offset field.
        field: Vector,
    },
    /// View height declaration.
    ViewHeight {
        /// Height scalar.
        height: Scalar,
    },
    /// Movement mode declaration.
    MovementMode {
        /// Mode field.
        field: Scalar,
        /// Field mask.
        mask: Option<f64>,
        /// Mode values.
        values: Vec<ModClientMovementModeValue>,
    },
    /// Stance declaration.
    Stance {
        /// Stance field.
        field: Scalar,
        /// Field mask.
        mask: Option<f64>,
        /// Stance values.
        values: Vec<ModClientStanceValue>,
    },
}

/// Movement mode value mapping.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModClientMovementModeValue {
    /// Raw value.
    pub value: f64,
    /// Movement mode.
    pub mode: ModClientMovementMode,
}

/// Stance value mapping.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModClientStanceValue {
    /// Raw value.
    pub value: f64,
    /// Crouched.
    pub crouched: bool,
}

/// Mod client movement outputs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModClientMovementOutputs {
    /// View offset.
    pub view_offset: Option<Vec3>,
    /// Movement mode.
    pub mode: Option<ModClientMovementMode>,
    /// Stance (crouched).
    pub stance: Option<bool>,
    /// Body bounds.
    pub body_bounds: Option<Bounds>,
}

// Boundary client presentation (donor `mod-client-presentation.ts`).

/// QuakeC mod client presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QcModClientPresentation {
    /// HUD presentation.
    pub hud: QcModHudPresentation,
    /// View presentation.
    pub view: QcModViewPresentation,
}

/// QuakeC mod HUD presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QcModHudPresentation {
    /// No HUD.
    None,
    /// Replace vitals.
    ReplaceVitals,
}

/// QuakeC mod view presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QcModViewPresentation {
    /// No view.
    None,
    /// Set view.
    SetView,
}

/// Native mod client presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeModClientPresentation {
    /// HUD presentation.
    pub hud: NativeModHudPresentation,
    /// View presentation.
    pub view: NativeModViewPresentation,
}

/// Native mod HUD presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeModHudPresentation {
    /// No HUD.
    None,
    /// Layout overlay.
    LayoutOverlay,
    /// Replace status.
    ReplaceStatus,
}

/// Native mod view presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeModViewPresentation {
    /// No view.
    None,
    /// Playerstate view.
    Playerstate,
}

// Boundary weapon stage (donor `qc-weapon-stage.ts`).

/// QuakeC source statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QcStatement {
    /// Opcode.
    pub opcode: u32,
    /// Operand A.
    pub a: u32,
    /// Operand B.
    pub b: u32,
    /// Operand C.
    pub c: u32,
}

/// QuakeC weapon stage declaration.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QcWeaponStageDeclaration {
    /// Dispatcher function.
    pub dispatcher: String,
    /// Continuation functions.
    pub continuations: Vec<String>,
    /// Repeat regions.
    pub repeats: Vec<QcWeaponStageRepeat>,
}

/// QuakeC weapon stage repeat region.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QcWeaponStageRepeat {
    /// Function name.
    pub function: String,
    /// Entry statement.
    pub entry: u32,
    /// Exit statement.
    pub exit: u32,
    /// Result word and value.
    pub result: QcWeaponStageResult,
    /// Exact source statements.
    pub statements: Vec<QcStatement>,
}

/// QuakeC weapon stage result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QcWeaponStageResult {
    /// Result word.
    pub word: u32,
    /// Result value (0 or 1).
    pub value: u8,
}

// `mod-callbacks.ts`.

/// QuakeC mod objective storage.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum QcModObjectiveStorage {
    /// Named storage.
    Named(String),
    /// Entity field storage.
    EntityField {
        /// Global name.
        global: String,
        /// Indirections.
        indirections: Vec<String>,
        /// Field name.
        field: String,
    },
}

/// Empty Quake armor declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct ModQcEmptyArmor {
    /// Armor item.
    pub item: ModQcEmptyArmorItem,
    /// Absorption.
    pub absorption: f64,
}

/// Empty Quake armor item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModQcEmptyArmorItem {
    /// Light armor.
    Armor1,
    /// Heavy armor.
    Armor2,
    /// Invulnerability armor.
    ArmorInv,
}

/// Mod client input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModClientInput {
    /// View angles.
    ViewAngles,
    /// Attack.
    Attack,
    /// Jump.
    Jump,
    /// Impulse.
    Impulse,
    /// Forward move.
    ForwardMove,
    /// Side move.
    SideMove,
    /// Up move.
    UpMove,
}

/// Non-view-angles client input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModClientScalarInput {
    /// Attack.
    Attack,
    /// Jump.
    Jump,
    /// Impulse.
    Impulse,
    /// Forward move.
    ForwardMove,
    /// Side move.
    SideMove,
    /// Up move.
    UpMove,
}

/// Mod client input/output.
#[derive(Debug, Clone, PartialEq)]
pub enum ModClientInputOutput {
    /// Set view angles.
    SetViewAngles(Vec3),
    /// Set a scalar input.
    SetScalar {
        /// Input.
        input: ModClientScalarInput,
        /// Value.
        value: f64,
    },
    /// Consume scalar inputs.
    Consume {
        /// Inputs.
        inputs: Vec<ModClientScalarInput>,
    },
}

/// Mod callback input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModCallbackInput {
    /// Client input.
    Client(ModClientInput),
    /// Self actor.
    Slf,
    /// Other actor.
    Other,
    /// Activator.
    Activator,
    /// Attacker.
    Attacker,
    /// Inflictor.
    Inflictor,
    /// Amount.
    Amount,
    /// Damage flags.
    DamageFlags,
    /// Regular protection scale.
    RegularProtectionScale,
    /// Knockback.
    Knockback,
    /// Point.
    Point,
    /// Direction.
    Direction,
    /// Normal.
    Normal,
    /// Item.
    Item,
    /// Time.
    Time,
    /// Elapsed.
    Elapsed,
    /// Result.
    Result,
    /// Pickup count.
    PickupCount,
    /// Pickup has count.
    PickupHasCount,
    /// Pickup dropped.
    PickupDropped,
}

/// Mod callback value.
#[derive(Debug, Clone, PartialEq)]
pub enum ModCallbackValue {
    /// Named input.
    Input(ModCallbackInput),
    /// Float constant.
    Float(f64),
    /// String constant.
    Str(ModCallbackString),
    /// Vector constant.
    Vector(Vec3),
}

/// String callback value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModCallbackString(pub String);

/// Mod actor field.
#[derive(Debug, Clone, PartialEq)]
pub struct ModActorField {
    /// Field name.
    pub field: String,
    /// Field binding.
    pub binding: ModActorFieldBinding,
}

/// Mod actor field binding.
#[derive(Debug, Clone, PartialEq)]
pub enum ModActorFieldBinding {
    /// Match field.
    Match(SourceMatchField),
    /// Health.
    Health,
    /// Origin.
    Origin,
    /// Velocity.
    Velocity,
    /// Angles.
    Angles,
    /// Bounds minimum.
    BoundsMin,
    /// Bounds maximum.
    BoundsMax,
    /// Think.
    Think,
    /// Next think.
    Nextthink,
    /// Private storage.
    Private,
    /// Classname.
    Classname,
    /// View offset.
    ViewOffset,
    /// Client flags.
    ClientFlags {
        /// Grounded.
        grounded: bool,
        /// Private mask.
        private_mask: Option<f64>,
    },
    /// Client input.
    ClientInput {
        /// Input.
        input: ModClientInput,
        /// Update mode.
        update: ModClientInputUpdate,
        /// Scale.
        scale: Option<f64>,
    },
    /// Userinfo key.
    Userinfo {
        /// Key.
        key: String,
    },
    /// Inventory item.
    Inventory {
        /// Item identifier.
        item: ItemId,
    },
    /// Constant value.
    Constant(ModRuntimeConstant),
}

/// Client input update mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModClientInputUpdate {
    /// Always update.
    Always,
    /// Update when nonzero.
    Nonzero,
}

/// Constant callback value.
#[derive(Debug, Clone, PartialEq)]
pub enum ModRuntimeConstant {
    /// Float constant.
    Float(f64),
    /// String constant.
    Str(ModCallbackString),
    /// Vector constant.
    Vector(Vec3),
}

/// Mod actor operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModActorOperation {
    /// Think.
    Think,
    /// Touch.
    Touch,
    /// Use.
    Use,
    /// Pain.
    Pain,
    /// Die.
    Die,
}

/// Mod callback operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModCallbackOperation {
    /// Actor operation.
    Actor(ModActorOperation),
    /// Damage.
    Damage,
    /// Inventory give.
    InventoryGive,
    /// Inventory consume.
    InventoryConsume,
}

/// Mod source call.
#[derive(Debug, Clone, PartialEq)]
pub struct ModSourceCall {
    /// Function name.
    pub function: String,
    /// Arguments.
    pub arguments: Vec<ModCallbackValue>,
    /// Globals.
    pub globals: Vec<ModCallbackGlobal>,
}

/// Mod callback global assignment.
#[derive(Debug, Clone, PartialEq)]
pub struct ModCallbackGlobal {
    /// Global name.
    pub name: String,
    /// Value.
    pub value: ModCallbackValue,
}

/// Mod callback binding.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModCallbackBinding {
    /// Binding identity.
    pub id: CallbackId,
    /// Operation plus stage.
    pub binding: ModCallbackBindingKind,
}

/// Mod callback operation plus stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModCallbackBindingKind {
    /// Observe stage.
    Observe {
        /// Operation.
        operation: ModCallbackOperation,
    },
    /// Damage transform stage.
    DamageTransform {
        /// Result.
        result: DamageTransformResult,
    },
    /// Inventory transform stage.
    InventoryTransform {
        /// Operation.
        operation: InventoryTransformOperation,
    },
    /// Actor replace stage.
    ActorReplace {
        /// Operation.
        operation: ModActorOperation,
    },
}

/// Damage transform result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DamageTransformResult {
    /// Amount.
    Amount,
    /// Knockback.
    Knockback,
}

/// Inventory transform operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InventoryTransformOperation {
    /// Give.
    Give,
    /// Consume.
    Consume,
}

/// Mod callback.
#[derive(Debug, Clone, PartialEq)]
pub struct ModCallback {
    /// Source call.
    pub call: ModSourceCall,
    /// Binding.
    pub binding: ModCallbackBinding,
}

/// Mod client input binding.
#[derive(Debug, Clone, PartialEq)]
pub struct ModClientInputBinding<Call, Output> {
    /// Binding scope.
    pub scope: ModClientInputScope,
    /// Calls.
    pub calls: Vec<Call>,
    /// Phase.
    pub phase: ModClientInputPhase<Output>,
}

/// Mod client input scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModClientInputScope {
    /// Client command.
    ClientCommand,
    /// Movement slice.
    MovementSlice,
}

/// Mod client input phase.
#[derive(Debug, Clone, PartialEq)]
pub enum ModClientInputPhase<Output> {
    /// Before phase.
    Before {
        /// Outputs.
        outputs: Vec<Output>,
    },
    /// After phase.
    After,
}

/// QuakeC mod input/output.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ModQcInputOutput {
    /// Field output.
    Field {
        /// Field name.
        field: String,
    },
    /// Handler output.
    Handler {
        /// Function name.
        function: String,
        /// Inputs.
        inputs: Vec<ModClientScalarInput>,
    },
}

/// Runtime callback value.
#[derive(Debug, Clone, PartialEq)]
pub enum ModRuntimeValue {
    /// Float constant.
    Float(f64),
    /// String constant.
    Str(ModCallbackString),
    /// Vector constant.
    Vector(Vec3),
    /// Actor value.
    Actor(Option<ActorId>),
}

/// Console callback value.
#[derive(Debug, Clone, PartialEq)]
pub enum ModConsoleValue {
    /// Float constant.
    Float(f64),
    /// String constant.
    Str(ModCallbackString),
    /// Vector constant.
    Vector(Vec3),
    /// Console argument.
    Argument {
        /// Argument index.
        index: u64,
        /// Argument type.
        r#type: ModConsoleArgumentType,
    },
    /// Arguments text.
    ArgumentsText,
    /// Argument count.
    ArgumentCount,
}

/// Console argument type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModConsoleArgumentType {
    /// String argument.
    Str,
    /// Float argument.
    Float,
}

/// Mod console command.
#[derive(Debug, Clone, PartialEq)]
pub struct ModConsoleCommand {
    /// Command name.
    pub name: String,
    /// Function name.
    pub function: String,
    /// Arguments.
    pub arguments: Vec<ModConsoleValue>,
    /// Globals.
    pub globals: Vec<ModConsoleGlobal>,
}

/// Mod console global assignment.
#[derive(Debug, Clone, PartialEq)]
pub struct ModConsoleGlobal {
    /// Global name.
    pub name: String,
    /// Value.
    pub value: ModConsoleValue,
}

/// QuakeC damage scale stage.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModQcDamageScale {
    /// Scale kind.
    pub kind: ModQcDamageScaleKind,
    /// Function name.
    pub function: String,
    /// Entry statement.
    pub entry: u32,
    /// Exit statement.
    pub exit: u32,
    /// Damage word.
    pub damage: u32,
    /// Exact source statements.
    pub statements: Vec<QcStatement>,
}

/// QuakeC damage scale kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModQcDamageScaleKind {
    /// Multiplier.
    Multiplier,
    /// Identity.
    Identity,
    /// Transform.
    Transform,
}

/// QuakeC armor stage.
#[derive(Debug, Clone, PartialEq)]
pub struct ModQcArmorStage {
    /// Function name.
    pub function: String,
    /// Entry statement.
    pub entry: u32,
    /// Exit statement.
    pub exit: u32,
    /// Target word.
    pub target: u32,
    /// Damage word.
    pub damage: u32,
    /// Saved word.
    pub saved: u32,
    /// Regular scale overrides.
    pub regular_scale: Vec<ModQcRegularScale>,
    /// Damage flags.
    pub flags: ModQcDamageFlags,
    /// Exact source statements.
    pub statements: Vec<QcStatement>,
}

/// Regular scale override.
#[derive(Debug, Clone, PartialEq)]
pub struct ModQcRegularScale {
    /// Caller function.
    pub caller: String,
    /// Statement index.
    pub statement: u32,
    /// Scale.
    pub scale: f64,
}

/// QuakeC damage flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModQcDamageFlags {
    /// No flags.
    None,
    /// Bit flags.
    Bits {
        /// Flag word.
        word: u32,
        /// No armor bit.
        no_armor: u32,
        /// No power armor bit.
        no_power_armor: u32,
        /// No regular armor bit.
        no_regular_armor: u32,
        /// Energy bit.
        energy: u32,
    },
}

/// QuakeC protection declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct ModQcProtection {
    /// Protection identity.
    pub id: String,
    /// Admission.
    pub admission: Option<ModProtectionAdmission>,
    /// Absorption.
    pub absorb: ModQcAbsorb,
    /// Flag bits.
    pub flags: ModQcProtectionFlags,
    /// Channel storage.
    pub channel: ModQcProtectionChannel,
}

/// Mod protection admission.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ModProtectionAdmission {
    /// Claim the channel.
    Claim,
    /// Replace the current primary.
    ReplaceCurrentPrimary,
    /// Replace an owner's primary.
    ReplacePrimary {
        /// Owner provider.
        owner: ProviderId,
    },
}

/// QuakeC absorption.
#[derive(Debug, Clone, PartialEq)]
pub enum ModQcAbsorb {
    /// Function absorption.
    Function {
        /// Absorb call.
        call: ModSourceCall,
    },
    /// Region absorption.
    Region {
        /// Absorb call.
        call: ModSourceCall,
        /// Armor stage.
        stage: ModQcArmorStage,
    },
}

/// QuakeC protection flag bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModQcProtectionFlags {
    /// No armor bit.
    pub no_armor: u32,
    /// No power armor bit.
    pub no_power_armor: u32,
    /// No regular armor bit.
    pub no_regular_armor: u32,
    /// Energy bit.
    pub energy: u32,
    /// Radius bit.
    pub radius: u32,
}

/// QuakeC protection channel storage.
#[derive(Debug, Clone, PartialEq)]
pub enum ModQcProtectionChannel {
    /// Regular channel.
    Regular {
        /// Armor points field.
        points: String,
        /// Armor item.
        item: Option<ItemId>,
        /// Selection.
        selection: Option<ModQcArmorSelection<Option<ItemId>>>,
    },
    /// Powered channel.
    Powered {
        /// Cells field.
        cells: String,
        /// Protection kind.
        kind: PoweredProtectionKind,
        /// Selection.
        selection: Option<ModQcArmorSelection<PoweredProtectionKind>>,
    },
}

/// Powered protection kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PoweredProtectionKind {
    /// No protection.
    None,
    /// Screen protection.
    Screen,
    /// Shield protection.
    Shield,
}

/// QuakeC armor selection.
#[derive(Debug, Clone, PartialEq)]
pub struct ModQcArmorSelection<Value> {
    /// Selection field.
    pub field: String,
    /// Selection mask.
    pub mask: Option<f64>,
    /// Selection values.
    pub values: Vec<ModQcArmorSelectionValue<Value>>,
}

/// QuakeC armor selection value.
#[derive(Debug, Clone, PartialEq)]
pub struct ModQcArmorSelectionValue<Value> {
    /// Raw value.
    pub value: f64,
    /// Selected value.
    pub selected: Value,
}

/// QuakeC mod items.
#[derive(Debug, Clone, PartialEq)]
pub struct ModQcItems {
    /// Item definitions.
    pub definitions: Vec<ModQcItemDefinition>,
    /// Item storage.
    pub storage: Vec<ModQcItemStorage>,
    /// Weapon stage.
    pub weapons: Option<ModQcWeaponStage>,
}

/// QuakeC mod item definition.
#[derive(Debug, Clone, PartialEq)]
pub struct ModQcItemDefinition {
    /// Item identifier.
    pub item: ItemId,
    /// Display label.
    pub label: String,
    /// Item icon.
    pub icon: Option<SourceItemIconDeclaration>,
    /// Admission mode.
    pub admission: SourceItemAdmissionMode,
    /// Action calls.
    pub actions: Option<SourceItemActionCalls<ModSourceCall>>,
    /// Item kind.
    pub kind: ModQcItemKind,
}

/// QuakeC mod item kind.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum ModQcItemKind {
    /// Counter item.
    Counter,
    /// Weapon item.
    Weapon(SourceWeaponItem),
}

/// QuakeC mod item storage.
#[derive(Debug, Clone, PartialEq)]
pub enum ModQcItemStorage {
    /// Counter storage.
    Counter {
        /// Storage field.
        field: String,
        /// Item identifier.
        item: ItemId,
        /// Capacity.
        capacity: ModQcItemCapacity,
    },
    /// Bit storage.
    Bits {
        /// Storage field.
        field: String,
        /// Private mask.
        private_mask: f64,
        /// Masked items.
        items: Vec<ModQcMaskedItem>,
    },
}

/// QuakeC mod item capacity.
#[derive(Debug, Clone, PartialEq)]
pub enum ModQcItemCapacity {
    /// Constant capacity.
    Constant {
        /// Value.
        value: f64,
    },
    /// Field capacity.
    Field {
        /// Field name.
        field: String,
    },
}

/// Masked storage item.
#[derive(Debug, Clone, PartialEq)]
pub struct ModQcMaskedItem {
    /// Item identifier.
    pub item: ItemId,
    /// Mask.
    pub mask: f64,
}

/// QuakeC mod weapon stage.
#[derive(Debug, Clone, PartialEq)]
pub struct ModQcWeaponStage {
    /// Stage declaration.
    pub stage: QcWeaponStageDeclaration,
    /// Selected weapon mapping.
    pub selected: ModQcWeaponMapping,
    /// Select weapon mapping.
    pub select: ModQcWeaponSelect,
    /// Resume calls.
    pub resume: Vec<ModSourceCall>,
    /// Weapon model fields.
    pub model: ModQcWeaponModel,
}

/// QuakeC weapon value mapping.
#[derive(Debug, Clone, PartialEq)]
pub struct ModQcWeaponMapping {
    /// Mapping field.
    pub field: String,
    /// Mapped values.
    pub values: Vec<ModQcWeaponValue>,
}

/// QuakeC weapon value.
#[derive(Debug, Clone, PartialEq)]
pub struct ModQcWeaponValue {
    /// Raw value.
    pub value: f64,
    /// Weapon item.
    pub item: ItemId,
}

/// QuakeC weapon select mapping.
#[derive(Debug, Clone, PartialEq)]
pub struct ModQcWeaponSelect {
    /// Mapping field.
    pub field: String,
    /// Mapped values.
    pub values: Vec<ModQcWeaponValue>,
    /// Select call.
    pub call: ModSourceCall,
}

/// QuakeC weapon model fields.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModQcWeaponModel {
    /// Model field.
    pub field: String,
    /// Frame field.
    pub frame: String,
}

/// Mod callback declaration (QuakeC runtime).
#[derive(Debug, Clone, PartialEq)]
pub struct ModCallbackDeclaration {
    /// Objective declarations.
    pub objectives: Vec<SourceObjectiveDeclaration<QcModObjectiveStorage, QcModObjectiveStorage, ModSourceCall>>,
    /// Client presentation.
    pub client_presentation: Option<QcModClientPresentation>,
    /// Schema version (1).
    pub version: u32,
    /// Program artifact.
    pub program: ModProgram,
    /// Actor fields.
    pub actor_fields: Vec<ModActorField>,
    /// Callbacks.
    pub callbacks: Vec<ModCallback>,
    /// Client bindings.
    pub clients: Option<ModQcClients>,
    /// Console variables.
    pub cvars: Vec<ModCvar>,
    /// Initialize calls.
    pub initialize: Vec<ModSourceCall>,
    /// Frame call.
    pub frame: Option<ModSourceCall>,
    /// Console commands.
    pub commands: Vec<ModConsoleCommand>,
    /// Combat lowering.
    pub combat: Option<ModQcCombat>,
    /// Protection declarations.
    pub protection: Vec<ModQcProtection>,
    /// Pickup rules.
    pub pickups: Vec<ModPickupRule<ModSourceCall>>,
    /// Item declarations.
    pub items: Option<ModQcItems>,
}

/// Mod program artifact.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModProgram {
    /// Program path.
    pub path: String,
    /// Program digest.
    pub digest: ContentDigest,
}

/// QuakeC mod client bindings.
#[derive(Debug, Clone, PartialEq)]
pub struct ModQcClients {
    /// Client outputs.
    pub outputs: Vec<ModClientOutputDeclaration<String, String>>,
    /// Maximum clients.
    pub maximum: u64,
    /// Admit calls.
    pub admit: Vec<ModSourceCall>,
    /// Userinfo calls.
    pub userinfo: Vec<ModSourceCall>,
    /// Disconnect calls.
    pub disconnect: Vec<ModSourceCall>,
    /// Frame calls.
    pub frame: Vec<ModSourceCall>,
    /// Input bindings.
    pub input: Vec<ModClientInputBinding<ModSourceCall, ModQcInputOutput>>,
}

/// Mod console variable.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModCvar {
    /// Variable name.
    pub name: String,
    /// Variable value.
    pub value: String,
}

/// QuakeC mod combat lowering.
#[derive(Debug, Clone, PartialEq)]
pub struct ModQcCombat {
    /// Damage call.
    pub damage: ModSourceCall,
    /// Damage scale stage.
    pub damage_scale: Option<ModQcDamageScale>,
    /// Armor stage.
    pub armor_stage: Option<ModQcArmorStage>,
    /// Empty armor.
    pub empty_armor: Option<ModQcEmptyArmor>,
}

// `mods.ts`.

/// Mod selection: package plus authored component.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModSelection {
    /// Product package.
    pub product: String,
    /// Component identity.
    pub id: String,
}

/// Mod description.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModDescription {
    /// Mod selection.
    pub selection: ModSelection,
    /// Defining provider.
    pub source: ProviderReference,
    /// Title.
    pub title: String,
    /// Source title.
    pub source_title: String,
    /// Purpose.
    pub purpose: ModPurpose,
    /// Required mods.
    pub requires: Vec<ModSelection>,
    /// Conflicting mods.
    pub conflicts: Vec<ModSelection>,
    /// Availability.
    pub availability: ModAvailability,
}

/// Mod purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModPurpose {
    /// Game type.
    GameType,
    /// Addition.
    Addition,
}

/// Mod availability.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ModAvailability {
    /// Available.
    Available,
    /// Unavailable.
    Unavailable {
        /// Reason.
        reason: String,
    },
}

/// Resolved gameplay mod.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedGameplayMod {
    /// Mod selection.
    pub selection: ModSelection,
    /// Defining provider.
    pub source: ProviderReference,
    /// Title.
    pub title: String,
    /// Source title.
    pub source_title: String,
    /// Required mods.
    pub requires: Vec<ModSelection>,
    /// Conflicting mods.
    pub conflicts: Vec<ModSelection>,
    /// Mod declaration.
    pub declaration: ModDeclaration,
    /// Declaration digest.
    pub declaration_digest: ContentDigest,
}

/// Gameplay mod declaration.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum ModDeclaration {
    /// QuakeC declaration.
    Quakec(ModCallbackDeclaration),
    /// QVM declaration.
    Qvm(QvmModCallbackDeclaration),
    /// Native declaration.
    Native(NativeModDeclaration),
}

/// Instance provider for a mod selection.
pub fn mod_instance_provider(selection: &ModSelection) -> Result<ProviderId, ContractError> {
    let key = mod_selection_key(selection)?;
    Ok(ProviderId::new("mod", &encode_uri_component(&key)))
}

/// Mod identity.
#[derive(Debug, Clone, PartialEq)]
pub struct ModIdentity {
    /// Mod selection.
    pub selection: ModSelection,
    /// Defining provider.
    pub source: ProviderReference,
    /// Declaration digest.
    pub declaration_digest: ContentDigest,
    /// Module identities.
    pub modules: Vec<ModuleIdentity>,
    /// Provider checkpoints without bytes.
    pub providers: Vec<ProviderCheckpointHeader>,
}

/// Provider checkpoint (donor `session.ts`).
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderCheckpoint {
    /// Provider identity.
    pub provider: ProviderId,
    /// Checkpoint schema.
    pub schema: String,
    /// Checkpoint version.
    pub version: f64,
    /// Checkpoint bytes.
    pub bytes: Vec<u8>,
}

/// Provider checkpoint header without bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderCheckpointHeader {
    /// Provider identity.
    pub provider: ProviderId,
    /// Checkpoint schema.
    pub schema: String,
    /// Checkpoint version.
    pub version: f64,
}

/// Mod private checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct ModPrivateCheckpoint {
    /// Guest checkpoints.
    pub guests: Vec<GuestCheckpoint>,
    /// Provider checkpoints.
    pub providers: Vec<ProviderCheckpoint>,
}

/// Mod checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct ModCheckpoint {
    /// Mod identity.
    pub identity: ModIdentity,
    /// Private state.
    pub state: ModPrivateCheckpoint,
}

/// Mod session checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct ModSessionCheckpoint {
    /// Schema version (1).
    pub version: u32,
    /// Mod checkpoints.
    pub mods: Vec<ModCheckpoint>,
}

/// Mod travel checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct ModTravelCheckpoint {
    /// Schema version (1).
    pub version: u32,
    /// Mod identities plus optional state.
    pub mods: Vec<ModTravelEntry>,
}

/// Mod travel entry.
#[derive(Debug, Clone, PartialEq)]
pub struct ModTravelEntry {
    /// Mod identity.
    pub identity: ModIdentity,
    /// Private state.
    pub state: Option<ModPrivateCheckpoint>,
}

/// Canonical `PRODUCT/COMPONENT_ID` selection key.
pub fn mod_selection_key(selection: &ModSelection) -> Result<String, ContractError> {
    if !identity_component(&selection.product) || !is_mod_component_id(&selection.id) {
        return Err(invalid(
            "Mod selection requires a package and an authored component ID".to_string(),
        ));
    }
    Ok(format!("{}/{}", selection.product, selection.id))
}

fn is_mod_component_id(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | ':' | '/' | '-'))
}

/// Read a `PRODUCT/COMPONENT_ID` selection.
pub fn read_mod_selection(value: &str) -> Result<ModSelection, ContractError> {
    let Some(slash) = value.find('/') else {
        return Err(invalid("Mod selection must be PRODUCT/COMPONENT_ID".to_string()));
    };
    if slash < 1 {
        return Err(invalid("Mod selection must be PRODUCT/COMPONENT_ID".to_string()));
    }
    let selection = ModSelection {
        product: value[..slash].to_string(),
        id: value[slash + 1..].to_string(),
    };
    if mod_selection_key(&selection)? != value {
        return Err(invalid("Mod selection must be PRODUCT/COMPONENT_ID".to_string()));
    }
    Ok(selection)
}

/// Compare mod identities.
#[must_use]
pub fn same_mod_identity(left: &ModIdentity, right: &ModIdentity) -> bool {
    let Ok(left_key) = mod_selection_key(&left.selection) else {
        return false;
    };
    let Ok(right_key) = mod_selection_key(&right.selection) else {
        return false;
    };
    left_key == right_key
        && left.source == right.source
        && left.declaration_digest == right.declaration_digest
        && left.modules == right.modules
        && left.providers == right.providers
}

// `equipment.ts`.

/// Shared grapple control.
pub trait SharedGrappleControl {
    /// Grapple selection.
    fn selection(&self) -> &GrappleSelection;
    /// Whether a mechanic uses the native slot.
    fn native_slot(&self, mechanic: GrappleMechanic) -> bool;
    /// Apply grapple input.
    fn input(&self, actor: &ActorId, held: bool);
    /// Release the grapple.
    fn release(&self, actor: &ActorId);
    /// Whether the grapple is pulling.
    fn pulling(&self, actor: &ActorId) -> bool;
    /// Gravity scale (0 or 1).
    fn gravity_scale(&self, actor: &ActorId) -> u8;
}

// Boundary QVM mod types (donors `qvm-mod-callbacks.ts`, `qvm-combat.ts`,
// `qvm-mod-actor-frame.ts`, `qvm-mod-items.ts`, `qvm-mod-presentation.ts`).

/// QVM mod scalar encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QvmModScalar {
    /// 32-bit integer.
    Int32,
    /// 32-bit float.
    Float32,
}

/// QVM mod value.
#[derive(Debug, Clone, PartialEq)]
pub enum QvmModValue {
    /// Scalar, vector or string value.
    Value {
        /// Value kind.
        kind: QvmModValueKind,
        /// Callback value.
        value: ModCallbackValue,
    },
    /// Actor value.
    Actor {
        /// Record name.
        record: String,
        /// Input actor.
        input: ModActorInput,
    },
    /// Client value.
    Client {
        /// Input actor.
        input: ModActorInput,
    },
    /// Time value.
    Time {
        /// Time input.
        input: ModTimeInput,
        /// Units.
        units: ModTimeUnits,
        /// Encoding.
        encoding: QvmModScalar,
    },
    /// Address value.
    Address(u32),
}

/// QVM mod value kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QvmModValueKind {
    /// 32-bit integer.
    Int32,
    /// 32-bit float.
    Float32,
    /// Vector.
    Vector,
    /// String.
    Str,
}

/// Mod actor input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModActorInput {
    /// Self actor.
    Slf,
    /// Other actor.
    Other,
    /// Activator.
    Activator,
    /// Attacker.
    Attacker,
    /// Inflictor.
    Inflictor,
}

/// Mod time input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModTimeInput {
    /// Time.
    Time,
    /// Elapsed.
    Elapsed,
}

/// Mod time units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModTimeUnits {
    /// Seconds.
    Seconds,
    /// Milliseconds.
    Milliseconds,
}

/// QVM mod source call.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModSourceCall {
    /// Entry instruction.
    pub entry: u32,
    /// Arguments.
    pub arguments: Vec<QvmModValue>,
    /// Globals.
    pub globals: Vec<QvmModGlobal>,
    /// Return encoding.
    pub returns: QvmModReturn,
}

/// QVM mod global assignment.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModGlobal {
    /// Global address.
    pub address: u32,
    /// Value.
    pub value: QvmModValue,
}

/// QVM mod return encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QvmModReturn {
    /// 32-bit integer.
    Int32,
    /// 32-bit float.
    Float32,
    /// Void.
    Void,
}

/// QVM mod pickup.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModPickup {
    /// Pickup rule.
    pub rule: ModPickupRule<QvmModSourceCall>,
    /// Projection context.
    pub context: Vec<QvmModPickupContext>,
}

/// QVM mod pickup context entry.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModPickupContext {
    /// Record name.
    pub record: String,
    /// Field offset.
    pub offset: u32,
    /// Value.
    pub value: QvmModValue,
}

/// QVM mod callback.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModCallback {
    /// Source call.
    pub call: QvmModSourceCall,
    /// Binding.
    pub binding: ModCallbackBinding,
}

/// QVM mod actor field.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModActorField {
    /// Field offset.
    pub offset: u32,
    /// Access mode.
    pub access: Option<QvmModFieldAccess>,
    /// Field binding.
    pub binding: QvmModActorFieldBinding,
}

/// QVM mod field access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QvmModFieldAccess {
    /// Read-write.
    ReadWrite,
    /// Read-only.
    ReadOnly,
}

/// QVM mod actor field binding.
#[derive(Debug, Clone, PartialEq)]
pub enum QvmModActorFieldBinding {
    /// Match field.
    Match {
        /// Field.
        field: SourceMatchField,
        /// Encoding.
        encoding: QvmModScalar,
    },
    /// Health.
    Health {
        /// Encoding.
        encoding: QvmModScalar,
    },
    /// Inventory item.
    Inventory {
        /// Encoding.
        encoding: QvmModScalar,
        /// Item identifier.
        item: ItemId,
    },
    /// Origin.
    Origin,
    /// Velocity.
    Velocity,
    /// Angles.
    Angles,
    /// Bounds minimum.
    BoundsMin,
    /// Bounds maximum.
    BoundsMax,
    /// Record link.
    Record {
        /// Record name.
        record: String,
    },
    /// Constant.
    Constant {
        /// Encoding.
        encoding: QvmModScalar,
        /// Value.
        value: f64,
    },
    /// Constant vector.
    ConstantVector(Vec3),
    /// Private storage.
    Private {
        /// Byte length.
        byte_length: u64,
    },
}

/// QVM mod actor record.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModActorRecord {
    /// Record identity.
    pub id: String,
    /// Record address.
    pub address: u32,
    /// Record stride.
    pub stride: u32,
    /// Record capacity.
    pub capacity: u64,
    /// Record fields.
    pub fields: Vec<QvmModActorField>,
}

/// QVM mod source actors.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModSourceActors {
    /// Allocate entry.
    pub allocate: u32,
    /// Release entry plus argument.
    pub release: QvmModRelease,
    /// In-use field offset.
    pub inuse: u32,
    /// Event entity type.
    pub event_entity_type: u32,
    /// Update call.
    pub update: Option<QvmModSourceCall>,
    /// Actor frame.
    pub frame: Option<QvmModActorFrame>,
    /// Initial stores.
    pub initial_stores: Vec<u32>,
    /// Entity callbacks.
    pub callbacks: Option<QvmModEntityCallbacks>,
}

/// QVM mod release entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QvmModRelease {
    /// Entry instruction.
    pub entry: u32,
    /// Argument.
    pub argument: u32,
}

/// QVM mod entity callbacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QvmModEntityCallbacks {
    /// Touch entry.
    pub touch: Option<u32>,
    /// Use entry.
    pub r#use: Option<u32>,
    /// Pain entry.
    pub pain: Option<u32>,
    /// Die entry.
    pub die: Option<u32>,
}

/// QVM mod actor frame (donor `qvm-mod-actor-frame.ts`).
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModActorFrame {
    /// Frame call.
    pub call: QvmModSourceCall,
    /// Frame clock.
    pub clock: QvmModActorClock,
    /// Owned instructions.
    pub owned: Vec<QvmModOwnedInstruction>,
    /// Loop end.
    pub end: QvmModActorEnd,
}

/// QVM mod actor clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QvmModActorClock {
    /// Clock address.
    pub address: u32,
    /// Store instruction.
    pub store: u32,
    /// Argument.
    pub argument: u32,
}

/// QVM mod owned instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QvmModOwnedInstruction {
    /// Instruction.
    pub instruction: u32,
    /// Local instruction.
    pub local_instruction: u32,
}

/// QVM mod actor loop end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QvmModActorEnd {
    /// End instruction.
    pub instruction: u32,
    /// Completed branch taken.
    pub completed_taken: bool,
}

/// QVM damage role (donor `qvm-combat.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QvmDamageRole {
    /// Target.
    Target,
    /// Inflictor.
    Inflictor,
    /// Attacker.
    Attacker,
    /// Direction.
    Direction,
    /// Point.
    Point,
    /// Amount.
    Amount,
    /// Flags.
    Flags,
    /// Method.
    Method,
}

/// QVM combat call.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmCombatCall<Role> {
    /// Role argument indices.
    pub roles: Vec<(Role, u32)>,
    /// Extra arguments.
    pub extras: Vec<QvmCombatExtra>,
}

/// QVM combat extra argument.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QvmCombatExtra {
    /// Argument index.
    pub index: u32,
    /// Argument kind.
    pub kind: QvmCombatExtraKind,
    /// Value.
    pub value: f64,
}

/// QVM combat extra kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QvmCombatExtraKind {
    /// 32-bit integer.
    Int32,
    /// 32-bit float.
    Float32,
    /// Address.
    Address,
}

/// QVM damage flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QvmDamageFlags {
    /// Radius flag.
    pub radius: u32,
    /// No armor flag.
    pub no_armor: u32,
    /// No knockback flag.
    pub no_knockback: u32,
    /// No protection flag.
    pub no_protection: u32,
    /// No team protection flag.
    pub no_team_protection: u32,
}

/// QVM combat mass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QvmCombatMass {
    /// Constant mass.
    Constant {
        /// Value.
        value: f64,
    },
    /// Entity mass.
    Entity {
        /// Field offset.
        offset: u32,
        /// Storage.
        storage: QvmCombatMassStorage,
    },
}

/// QVM combat mass storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QvmCombatMassStorage {
    /// 32-bit integer.
    Int32,
    /// 32-bit float.
    Float32,
}

/// QVM combat team.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmCombatTeam {
    /// Raw value.
    pub value: f64,
    /// Team identity.
    pub team: String,
}

/// QVM mod combat calls.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModCombatCalls {
    /// Damage call.
    pub damage: QvmCombatCall<QvmDamageRole>,
    /// Touch call.
    pub touch: QvmCombatCall<QvmTouchRole>,
    /// Use call.
    pub r#use: QvmCombatCall<QvmUseRole>,
    /// Pain call.
    pub pain: QvmCombatCall<QvmPainRole>,
    /// Die call.
    pub die: QvmCombatCall<QvmDieRole>,
}

/// QVM touch roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QvmTouchRole {
    /// Target.
    Target,
    /// Other.
    Other,
    /// Trace.
    Trace,
}

/// QVM use roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QvmUseRole {
    /// Target.
    Target,
    /// Other.
    Other,
    /// Activator.
    Activator,
}

/// QVM pain roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QvmPainRole {
    /// Target.
    Target,
    /// Attacker.
    Attacker,
    /// Amount.
    Amount,
}

/// QVM die roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QvmDieRole {
    /// Target.
    Target,
    /// Inflictor.
    Inflictor,
    /// Attacker.
    Attacker,
    /// Amount.
    Amount,
    /// Method.
    Method,
}

/// QVM mod combat.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModCombat {
    /// Combat entry.
    pub entry: u32,
    /// Health field offset.
    pub health: u32,
    /// Take-damage field offset.
    pub takedamage: u32,
    /// Flags field offset.
    pub flags: u32,
    /// Godmode field offset.
    pub godmode: u32,
    /// No-knockback field offset.
    pub no_knockback: u32,
    /// Globals.
    pub globals: Vec<QvmModGlobal>,
    /// Client projection.
    pub client: Option<QvmModCombatClient>,
    /// Combat ABI.
    pub abi: QvmModCombatAbi,
}

/// QVM mod combat client projection.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QvmModCombatClient {
    /// Client pointer.
    pub pointer: u32,
    /// Record name.
    pub record: String,
    /// Health field offset.
    pub health: u32,
    /// Armor field offset.
    pub armor: u32,
    /// Protection field offset.
    pub protection: u32,
    /// Team field offset.
    pub team: u32,
}

/// QVM mod combat ABI.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum QvmModCombatAbi {
    /// Original G_Damage ABI.
    GDamage,
    /// Declared calls.
    Declared {
        /// Combat calls.
        calls: QvmModCombatCalls,
        /// Damage flags.
        damage_flags: QvmDamageFlags,
        /// Combat mass.
        mass: QvmCombatMass,
        /// Combat teams.
        teams: Vec<QvmCombatTeam>,
    },
}

/// QVM mod protection scalar.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QvmModProtectionScalar {
    /// Record name.
    pub record: String,
    /// Field offset.
    pub offset: u32,
    /// Encoding.
    pub encoding: QvmModScalar,
}

/// QVM mod protection selection.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModProtectionSelection<Value> {
    /// Selection field.
    pub field: QvmModProtectionScalar,
    /// Selection mask.
    pub mask: Option<f64>,
    /// Selection values.
    pub values: Vec<QvmModProtectionValue<Value>>,
}

/// QVM mod protection value.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModProtectionValue<Value> {
    /// Raw value.
    pub value: f64,
    /// Selected value.
    pub selected: Value,
}

/// QVM mod protection.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModProtection {
    /// Protection identity.
    pub id: String,
    /// Admission.
    pub admission: ModProtectionAdmission,
    /// Absorb call.
    pub absorb: QvmModSourceCall,
    /// Flag bits.
    pub flags: ModQcProtectionFlags,
    /// Channel storage.
    pub channel: QvmModProtectionChannel,
}

/// QVM mod protection channel.
#[derive(Debug, Clone, PartialEq)]
pub enum QvmModProtectionChannel {
    /// Regular channel.
    Regular {
        /// Armor points.
        points: QvmModProtectionScalar,
        /// Armor item.
        item: Option<ItemId>,
        /// Selection.
        selection: Option<QvmModProtectionSelection<Option<ItemId>>>,
    },
    /// Powered channel.
    Powered {
        /// Cells.
        cells: QvmModProtectionScalar,
        /// Selection.
        selection: QvmModProtectionSelection<PoweredProtectionKind>,
    },
}

/// QVM mod clients.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModClients {
    /// Client outputs.
    pub outputs: Vec<ModClientOutputDeclaration<QvmModProtectionScalar, QvmItemField>>,
    /// Maximum clients.
    pub maximum: u64,
    /// Records.
    pub records: Vec<String>,
    /// Player state record.
    pub player_state_record: String,
    /// Admit calls.
    pub admit: Vec<QvmModSourceCall>,
    /// Userinfo calls.
    pub userinfo: Vec<QvmModSourceCall>,
    /// Disconnect calls.
    pub disconnect: Vec<QvmModSourceCall>,
    /// Frame calls.
    pub frame: Vec<QvmModSourceCall>,
    /// Input bindings.
    pub input: Vec<ModClientInputBinding<QvmModSourceCall, QvmModInputOutput>>,
}

/// QVM mod input pointer.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QvmModInputPointer {
    /// Pointer base.
    pub base: QvmModInputPointerBase,
    /// Indirections.
    pub indirections: Vec<u32>,
    /// Offset.
    pub offset: u32,
}

/// QVM mod input pointer base.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QvmModInputPointerBase {
    /// Call argument.
    Argument {
        /// Argument index.
        index: u32,
    },
    /// Global address.
    Global {
        /// Address.
        address: u32,
    },
}

/// QVM mod objective address.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum QvmModObjectiveAddress {
    /// Instruction address.
    Address(u32),
    /// Global pointer.
    Global {
        /// Global address.
        address: u32,
        /// Indirections.
        indirections: Vec<u32>,
        /// Offset.
        offset: u32,
    },
}

/// QVM mod input/output.
#[derive(Debug, Clone, PartialEq)]
pub enum QvmModInputOutput {
    /// Field output.
    Field {
        /// Record name.
        record: String,
        /// Field offset.
        offset: u32,
        /// Input value.
        value: QvmModFieldInput,
    },
    /// Handler output.
    Handler {
        /// Handler entry.
        entry: u32,
        /// Handler actor.
        actor: QvmWeaponActor,
        /// Inputs.
        inputs: Vec<ModClientScalarInput>,
        /// Return expectation.
        returns: Option<QvmModHandlerReturn>,
    },
    /// Command output.
    Command {
        /// Handler entry.
        entry: u32,
        /// Handler actor.
        actor: QvmWeaponActor,
        /// Command pointer.
        command: QvmModInputPointer,
        /// Inputs (all but impulse).
        inputs: Vec<ModClientCommandInput>,
    },
}

/// QVM mod field input value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QvmModFieldInput {
    /// View angles.
    ViewAngles,
    /// Scalar input.
    Scalar {
        /// Input.
        input: ModClientScalarInput,
        /// Encoding.
        encoding: QvmModScalar,
        /// Scale.
        scale: f64,
    },
}

/// QVM mod handler return expectation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QvmModHandlerReturn {
    /// Encoding.
    pub encoding: QvmModScalar,
    /// Value.
    pub value: f64,
}

/// Client input other than impulse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModClientCommandInput {
    /// View angles.
    ViewAngles,
    /// Attack.
    Attack,
    /// Jump.
    Jump,
    /// Forward move.
    ForwardMove,
    /// Side move.
    SideMove,
    /// Up move.
    UpMove,
}

/// QVM item field (donor `qvm-mod-items.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QvmItemField {
    /// Record name.
    pub record: String,
    /// Field offset.
    pub offset: u32,
}

/// QVM item test.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmItemTest {
    /// Tested field.
    pub field: QvmItemField,
    /// Test mask.
    pub mask: Option<f64>,
    /// Comparison.
    pub comparison: ItemTestComparison,
    /// Value.
    pub value: f64,
}

/// Item test comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemTestComparison {
    /// Equals.
    Equals,
    /// At most.
    AtMost,
}

/// QVM item capacity.
#[derive(Debug, Clone, PartialEq)]
pub enum QvmItemCapacity {
    /// Constant capacity.
    Constant {
        /// Value.
        value: f64,
    },
    /// Field capacity.
    Field {
        /// Field.
        field: QvmItemField,
    },
    /// Source capacity.
    Source {
        /// Instruction.
        instruction: u32,
        /// Overrides.
        overrides: Vec<QvmItemCapacityOverride>,
    },
}

/// QVM item capacity override.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmItemCapacityOverride {
    /// Address.
    pub address: u32,
    /// Comparison.
    pub comparison: ItemCapacityComparison,
    /// Value.
    pub value: f64,
    /// Instruction.
    pub instruction: u32,
}

/// Item capacity comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemCapacityComparison {
    /// Equals.
    Equals,
    /// Not equals.
    NotEquals,
}

/// QVM item storage.
#[derive(Debug, Clone, PartialEq)]
pub enum QvmItemStorage {
    /// Counter storage.
    Counter {
        /// Storage field.
        field: QvmItemField,
        /// Item identifier.
        item: ItemId,
        /// Capacity.
        capacity: QvmItemCapacity,
    },
    /// Bit storage.
    Bits {
        /// Storage field.
        field: QvmItemField,
        /// Private mask.
        private_mask: f64,
        /// Masked items.
        items: Vec<QvmMaskedItem>,
    },
}

/// Masked QVM storage item.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmMaskedItem {
    /// Item identifier.
    pub item: ItemId,
    /// Mask.
    pub mask: f64,
}

/// QVM weapon actor.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QvmWeaponActor {
    /// Record name.
    pub record: String,
    /// Actor pointer.
    pub pointer: QvmModInputPointer,
}

/// QVM weapon stage.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmWeaponStage {
    /// Dispatcher.
    pub dispatcher: QvmWeaponDispatcher,
    /// Conditional predicates.
    pub predicates: Vec<QvmWeaponPredicate>,
    /// Settled tests.
    pub settled: Vec<QvmItemTest>,
    /// Selection.
    pub selection: QvmWeaponSelection,
    /// Request.
    pub request: QvmWeaponRequest,
    /// Continuation.
    pub continuation: QvmWeaponContinuation,
}

/// QVM weapon dispatcher.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QvmWeaponDispatcher {
    /// Entry instruction.
    pub entry: u32,
    /// Dispatcher actor.
    pub actor: QvmWeaponActor,
}

/// QVM weapon predicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QvmWeaponPredicate {
    /// Instruction.
    pub instruction: u32,
    /// Unselected branch.
    pub unselected: bool,
}

/// QVM weapon selection.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmWeaponSelection {
    /// Selection field.
    pub field: QvmItemField,
    /// Selection values.
    pub values: Vec<QvmWeaponValue>,
}

/// QVM weapon value.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmWeaponValue {
    /// Raw value.
    pub value: f64,
    /// Weapon item.
    pub item: ItemId,
}

/// QVM weapon request.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmWeaponRequest {
    /// Entry instruction.
    pub entry: u32,
    /// Argument.
    pub argument: u32,
    /// Accepted tests.
    pub accepted: Vec<QvmItemTest>,
}

/// QVM weapon continuation.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmWeaponContinuation {
    /// Entry instruction.
    pub entry: u32,
    /// Continuation actor.
    pub actor: QvmWeaponActor,
    /// Instruction.
    pub instruction: u32,
    /// Original branch taken.
    pub original_taken: bool,
    /// Guard tests.
    pub when: Vec<QvmItemTest>,
    /// Predicates.
    pub predicates: Vec<QvmWeaponPredicate>,
    /// Movement projection.
    pub projection: QvmWeaponProjection,
    /// Calls.
    pub calls: Vec<QvmWeaponCall>,
}

/// QVM weapon movement projection.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QvmWeaponProjection {
    /// Movement pointer.
    pub movement: QvmModInputPointer,
    /// Byte length.
    pub byte_length: u64,
    /// Minimum.
    pub minimum: u32,
    /// Maximum.
    pub maximum: u32,
    /// View height field.
    pub view_height: QvmItemField,
    /// Ground field.
    pub ground: QvmItemField,
}

/// QVM weapon continuation call.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmWeaponCall {
    /// Instruction.
    pub instruction: u32,
    /// Call.
    pub call: QvmModSourceCall,
}

/// QVM mod items.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModItems {
    /// Item definitions.
    pub definitions: Vec<QvmItemDefinition>,
    /// Item storage.
    pub storage: Vec<QvmItemStorage>,
    /// Weapon stage.
    pub weapons: Option<QvmItemsWeaponStage>,
}

/// QVM item definition.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmItemDefinition {
    /// Item identifier.
    pub item: ItemId,
    /// Display label.
    pub label: String,
    /// Item icon.
    pub icon: Option<SourceItemIconDeclaration>,
    /// Admission mode.
    pub admission: SourceItemAdmissionMode,
    /// Action calls.
    pub actions: Option<SourceItemActionCalls<QvmModSourceCall>>,
    /// Item kind.
    pub kind: QvmItemKind,
}

/// QVM item kind.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum QvmItemKind {
    /// Counter item.
    Counter,
    /// Weapon item.
    Weapon(SourceWeaponItem),
}

/// QVM items weapon stage.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmItemsWeaponStage {
    /// Input entry plus clock.
    pub input: QvmItemsWeaponInput,
    /// Weapon stage.
    pub stage: QvmWeaponStage,
}

/// QVM items weapon input.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QvmItemsWeaponInput {
    /// Entry instruction.
    pub entry: u32,
    /// Clock field.
    pub clock: QvmItemField,
}

/// QVM presentation program (donor `qvm-mod-presentation.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QvmPresentationProgram {
    /// Program path.
    pub path: String,
    /// Program digest.
    pub digest: ContentDigest,
    /// ABI profile.
    pub abi_profile: QvmAbiProfile,
}

/// QVM body part.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QvmBodyPart {
    /// Body.
    Body,
    /// Lower.
    Lower,
    /// Upper.
    Upper,
    /// Head.
    Head,
}

/// QVM presentation argument.
#[derive(Debug, Clone, PartialEq)]
pub enum QvmPresentationArgument {
    /// Immediate value.
    Immediate {
        /// Immediate kind.
        kind: QvmPresentationImmediateKind,
        /// Value.
        value: f64,
    },
    /// Caller-supplied source value.
    Source(QvmPresentationSource),
}

/// QVM presentation immediate kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QvmPresentationImmediateKind {
    /// 32-bit integer.
    Int32,
    /// 32-bit float.
    Float32,
    /// Address.
    Address,
}

/// QVM presentation caller-supplied source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QvmPresentationSource {
    /// Player state.
    PlayerState,
    /// Entity state.
    EntityState,
    /// Client entity.
    Centity,
    /// Origin.
    Origin,
    /// Snapshot.
    Snapshot,
    /// Client number.
    ClientNumber,
    /// Time.
    Time,
    /// Event.
    Event,
    /// Parameter.
    Parameter,
    /// Snapshot number.
    SnapshotNumber,
    /// Server command sequence.
    ServerCommandSequence,
}

/// QVM presentation call.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmPresentationCall {
    /// Entry instruction.
    pub entry: u32,
    /// Call timing.
    pub when: Option<QvmPresentationTiming>,
    /// Arguments.
    pub arguments: Vec<QvmPresentationArgument>,
}

/// QVM presentation call timing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QvmPresentationTiming {
    /// When the weapon is presented.
    WeaponPresented,
}

/// QVM presentation base.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmPresentationBase {
    /// Schema version (1).
    pub version: u32,
    /// Gameplay program.
    pub gameplay: QvmPresentationProgram,
    /// Client game program.
    pub cgame: QvmPresentationProgram,
    /// Initialize calls.
    pub initialize: Vec<QvmPresentationCall>,
    /// Refresh calls.
    pub refresh: Vec<QvmPresentationCall>,
    /// Frame calls.
    pub frame: Vec<QvmPresentationCall>,
    /// HUD presentation.
    pub hud: Option<QvmPresentationHud>,
}

/// QVM presentation HUD.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmPresentationHud {
    /// HUD mode.
    pub mode: QvmPresentationHudMode,
    /// Frame calls.
    pub frame: Vec<QvmPresentationCall>,
}

/// QVM presentation HUD mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QvmPresentationHudMode {
    /// Overlay.
    Overlay,
    /// Replace status.
    ReplaceStatus,
}

/// QVM player event presentation.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmPlayerEventPresentation {
    /// Presentation base.
    pub base: QvmPresentationBase,
    /// Storage addresses.
    pub storage: QvmPlayerEventStorage,
    /// Project calls.
    pub project: Vec<QvmPresentationCall>,
    /// Event call.
    pub event: QvmPresentationCall,
}

/// QVM player event storage.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QvmPlayerEventStorage {
    /// Game state address.
    pub game_state: u32,
    /// Player state address.
    pub player_state: u32,
    /// Synthetic snapshot.
    pub snapshot: QvmSyntheticSnapshot,
    /// Client entities.
    pub centities: QvmCentities,
    /// Time addresses.
    pub time: Vec<u32>,
    /// Frame time addresses.
    pub frame_time: Vec<u32>,
    /// View origin addresses.
    pub view_origin: Vec<u32>,
    /// View angles addresses.
    pub view_angles: Vec<u32>,
    /// View axis addresses.
    pub view_axis: Vec<u32>,
}

/// QVM synthetic snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QvmSyntheticSnapshot {
    /// Snapshot address.
    pub address: u32,
    /// Snapshot pointers.
    pub pointers: Vec<u32>,
}

/// QVM client entity table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QvmCentities {
    /// Table address.
    pub address: u32,
    /// Entry stride.
    pub stride: u32,
    /// Table capacity.
    pub capacity: u64,
    /// State offset.
    pub state: u32,
    /// Origin offset.
    pub origin: u32,
}

/// QVM scene presentation.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmScenePresentation {
    /// Presentation base.
    pub base: QvmPresentationBase,
    /// Console variables.
    pub cvars: Vec<ModCvar>,
    /// Storage addresses.
    pub storage: QvmSceneStorage,
    /// Snapshot calls.
    pub snapshots: Vec<QvmPresentationCall>,
    /// Event entity type.
    pub event_entity_type: u32,
    /// Event check.
    pub event_check: QvmEventCheck,
    /// Body presentation.
    pub body: QvmBodyPresentation,
}

/// QVM scene storage.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QvmSceneStorage {
    /// Game state address.
    pub game_state: u32,
    /// Server command sequence address.
    pub server_command_sequence: u32,
    /// Time addresses.
    pub time: Vec<u32>,
    /// Frame time addresses.
    pub frame_time: Vec<u32>,
    /// View origin addresses.
    pub view_origin: Vec<u32>,
    /// View angles addresses.
    pub view_angles: Vec<u32>,
    /// View axis addresses.
    pub view_axis: Vec<u32>,
    /// Client entities.
    pub centities: QvmSceneCentities,
}

/// QVM scene client entity table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QvmSceneCentities {
    /// Table address.
    pub address: u32,
    /// Entry stride.
    pub stride: u32,
    /// Table capacity.
    pub capacity: u64,
    /// State offset.
    pub state: u32,
    /// Previous event offset.
    pub previous_event: u32,
    /// Snapshot time offset.
    pub snapshot_time: u32,
}

/// QVM event check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QvmEventCheck {
    /// Entry instruction.
    pub entry: u32,
    /// Client entity argument.
    pub centity_argument: u32,
}

/// QVM body presentation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QvmBodyPresentation {
    /// Player presentation.
    pub player: QvmPlayerPresentation,
    /// Mesh presentation.
    pub mesh: QvmMeshPresentation,
}

/// QVM player presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QvmPlayerPresentation {
    /// Entry instruction.
    pub entry: u32,
    /// Client entity argument.
    pub centity_argument: u32,
}

/// QVM mesh presentation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QvmMeshPresentation {
    /// Entry instruction.
    pub entry: u32,
    /// Entity argument.
    pub entity_argument: u32,
    /// State argument.
    pub state_argument: u32,
    /// Shader offset.
    pub shader_offset: u32,
    /// Body part call sites.
    pub parts: Vec<QvmMeshPart>,
}

/// QVM mesh body part call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QvmMeshPart {
    /// Call instruction.
    pub call: u32,
    /// Body part.
    pub part: QvmBodyPart,
}

/// QVM mod presentation declaration.
#[derive(Debug, Clone, PartialEq)]
pub enum QvmModPresentationDeclaration {
    /// Player event presentation.
    PlayerEvents(QvmPlayerEventPresentation),
    /// Scene presentation.
    Scene(QvmScenePresentation),
}

/// QVM mod callback declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModCallbackDeclaration {
    /// Objective declarations.
    pub objectives: Vec<SourceObjectiveDeclaration<QvmModObjectiveStorage, QvmModObjectiveAddress, QvmModSourceCall>>,
    /// Schema version (1).
    pub version: u32,
    /// Program artifact.
    pub program: ModProgram,
    /// ABI profile.
    pub abi_profile: QvmAbiProfile,
    /// Presentation declaration.
    pub presentation: Option<QvmModPresentationDeclaration>,
    /// Spawn entities.
    pub spawn_entities: Option<String>,
    /// Client bindings.
    pub clients: Option<QvmModClients>,
    /// Actor records.
    pub actor_records: Vec<QvmModActorRecord>,
    /// Entity record.
    pub entity_record: Option<String>,
    /// Source actors.
    pub source_actors: Option<QvmModSourceActors>,
    /// Combat.
    pub combat: Option<QvmModCombat>,
    /// Protection declarations.
    pub protection: Vec<QvmModProtection>,
    /// Pickup rules.
    pub pickups: Vec<QvmModPickup>,
    /// Item declarations.
    pub items: Option<QvmModItems>,
    /// Initialize calls.
    pub initialize: Vec<QvmModSourceCall>,
    /// Callbacks.
    pub callbacks: Vec<QvmModCallback>,
}

/// QVM mod objective storage.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QvmModObjectiveStorage {
    /// Storage address.
    pub address: QvmModObjectiveAddress,
    /// Encoding.
    pub encoding: QvmModScalar,
}

// `native-mod-callbacks.ts`.

/// Native mod scalar encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeModScalar {
    /// Signed 8-bit.
    Int8,
    /// Unsigned 8-bit.
    Uint8,
    /// Signed 16-bit.
    Int16,
    /// Unsigned 16-bit.
    Uint16,
    /// Signed 32-bit.
    Int32,
    /// Unsigned 32-bit.
    Uint32,
    /// Signed 64-bit.
    Int64,
    /// Unsigned 64-bit.
    Uint64,
    /// 32-bit float.
    Float32,
    /// 64-bit float.
    Float64,
}

/// Native mod image-relative address.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NativeModAddress {
    /// Relative virtual address.
    pub rva: u64,
    /// Indirections.
    pub indirections: Vec<u64>,
}

/// Native mod entry point.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NativeModEntry {
    /// Named export.
    Export {
        /// Export name.
        name: String,
    },
    /// Relative virtual address.
    Rva {
        /// Relative virtual address.
        rva: u64,
    },
    /// Game export.
    GameExport {
        /// Export name.
        name: String,
    },
}

/// Native mod value.
#[derive(Debug, Clone, PartialEq)]
pub enum NativeModValue {
    /// Scalar, vector or string value.
    Value {
        /// Value kind.
        kind: NativeModValueKind,
        /// Callback value.
        value: ModCallbackValue,
    },
    /// Actor value.
    Actor {
        /// Record name.
        record: String,
        /// Input actor.
        input: ModActorInput,
    },
    /// Client value.
    Client {
        /// Input actor.
        input: ModActorInput,
    },
    /// Userinfo value.
    Userinfo {
        /// Input actor.
        input: ModActorInput,
    },
    /// User command value.
    UserCommand,
    /// Time value.
    Time {
        /// Time input.
        input: ModTimeInput,
        /// Units.
        units: ModTimeUnits,
        /// Encoding.
        encoding: NativeModScalar,
    },
    /// Address value.
    Address(Option<NativeModAddress>),
}

/// Native mod value kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeModValueKind {
    /// Scalar encoding.
    Scalar(NativeModScalar),
    /// Vector.
    Vector,
    /// String.
    Str,
}

/// Native mod source call.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeModSourceCall {
    /// Call entry.
    pub entry: NativeModEntry,
    /// Arguments.
    pub arguments: Vec<NativeModValue>,
    /// Globals.
    pub globals: Vec<NativeModGlobal>,
    /// Return encoding.
    pub returns: NativeModReturn,
    /// Excluded frame regions.
    pub skips: Vec<NativeModSkip>,
}

/// Native mod global assignment.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeModGlobal {
    /// Global address.
    pub address: NativeModAddress,
    /// Value.
    pub value: NativeModValue,
}

/// Native mod return encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeModReturn {
    /// Scalar encoding.
    Scalar(NativeModScalar),
    /// Void.
    Void,
}

/// Native mod excluded frame region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeModSkip {
    /// Entry offset.
    pub entry: u64,
    /// Join offset.
    pub join: u64,
}

/// Native mod callback.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeModCallback {
    /// Source call.
    pub call: NativeModSourceCall,
    /// Binding.
    pub binding: ModCallbackBinding,
}

/// Native mod pickup.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeModPickup {
    /// Pickup rule.
    pub rule: ModPickupRule<NativeModSourceCall>,
    /// Projection context.
    pub context: Vec<NativeModPickupContext>,
}

/// Native mod pickup context entry.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeModPickupContext {
    /// Record name.
    pub record: String,
    /// Field offset.
    pub offset: u64,
    /// Value.
    pub value: NativeModPickupValue,
}

/// Native mod pickup context value.
#[derive(Debug, Clone, PartialEq)]
pub enum NativeModPickupValue {
    /// Scalar or vector value.
    Value {
        /// Value kind.
        kind: NativeModScalarValueKind,
        /// Callback value.
        value: ModCallbackValue,
    },
    /// Time value.
    Time {
        /// Time input.
        input: ModTimeInput,
        /// Units.
        units: ModTimeUnits,
        /// Encoding.
        encoding: NativeModScalar,
    },
    /// Address value.
    Address(Option<NativeModAddress>),
}

/// Native mod scalar or vector value kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeModScalarValueKind {
    /// Scalar encoding.
    Scalar(NativeModScalar),
    /// Vector.
    Vector,
}

/// Native mod admission call.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeModAdmissionCall {
    /// Source call.
    pub call: NativeModSourceCall,
    /// Acceptance.
    pub accepts: NativeModAcceptance,
}

/// Native mod acceptance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeModAcceptance {
    /// Always accepts.
    Always,
    /// Nonzero accepts.
    Nonzero,
}

/// Native mod client input field.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeModClientInputField {
    /// Record name.
    pub record: String,
    /// Field offset.
    pub offset: u64,
    /// Value.
    pub value: NativeModClientInputValue,
}

/// Native mod client input value.
#[derive(Debug, Clone, PartialEq)]
pub enum NativeModClientInputValue {
    /// Scalar or vector value.
    Value {
        /// Value kind.
        kind: NativeModScalarValueKind,
        /// Callback value.
        value: ModCallbackValue,
    },
    /// Time value.
    Time {
        /// Time input.
        input: ModTimeInput,
        /// Units.
        units: ModTimeUnits,
        /// Encoding.
        encoding: NativeModScalar,
    },
}

/// Native mod input/output.
#[derive(Debug, Clone, PartialEq)]
pub enum NativeModInputOutput {
    /// Field output.
    Field {
        /// Record name.
        record: String,
        /// Field offset.
        offset: u64,
    },
    /// Handler output.
    Handler {
        /// Handler entry.
        entry: NativeModEntry,
        /// Arguments.
        arguments: Vec<NativeModValue>,
        /// Inputs.
        inputs: Vec<ModClientScalarInput>,
    },
}

/// Native mod scalar field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeModScalarField {
    /// Field offset.
    pub offset: u64,
    /// Encoding.
    pub encoding: NativeModScalar,
}

/// Native mod armor field.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NativeModArmorField {
    /// Field offset.
    pub offset: u64,
    /// Encoding.
    pub encoding: NativeModScalar,
    /// Record name.
    pub record: String,
}

/// Native mod clients.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeModClients {
    /// Client outputs.
    pub outputs: Vec<ModClientOutputDeclaration<NativeModArmorField, NativeItemPointer>>,
    /// Maximum clients.
    pub maximum: u64,
    /// Records.
    pub records: Vec<String>,
    /// Admit calls.
    pub admit: Vec<NativeModAdmissionCall>,
    /// Userinfo calls.
    pub userinfo: Vec<NativeModSourceCall>,
    /// Disconnect calls.
    pub disconnect: Vec<NativeModSourceCall>,
    /// Command calls.
    pub command: Vec<NativeModSourceCall>,
    /// Input bindings.
    pub input: Vec<ModClientInputBinding<NativeModSourceCall, NativeModInputOutput>>,
    /// Frame calls.
    pub frame: Vec<NativeModSourceCall>,
    /// End-frame calls.
    pub end_frame: Vec<NativeModSourceCall>,
    /// Input fields.
    pub input_fields: Vec<NativeModClientInputField>,
    /// Pose projection.
    pub pose: Option<NativeModPose>,
}

/// Native mod pose projection.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeModPose {
    /// View height field.
    pub view_height: NativeModArmorField,
    /// Crouched test.
    pub crouched: NativeModMaskedField,
}

/// Masked native field test.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeModMaskedField {
    /// Tested field.
    pub field: NativeModArmorField,
    /// Mask.
    pub mask: f64,
}

/// Native mod actor field.
#[derive(Debug, Clone, PartialEq)]
pub enum NativeModActorField {
    /// QVM-style field (excluding health, inventory, constant, team, score).
    Shared(NativeModSharedActorField),
    /// Address binding.
    Address {
        /// Field offset.
        offset: u64,
        /// Value.
        value: Option<NativeModAddress>,
    },
    /// Match binding.
    Match {
        /// Field offset.
        offset: u64,
        /// Encoding.
        encoding: NativeModScalar,
        /// Field.
        field: SourceMatchField,
    },
    /// Health binding.
    Health {
        /// Field offset.
        offset: u64,
        /// Encoding.
        encoding: NativeModScalar,
    },
    /// Inventory binding.
    Inventory {
        /// Field offset.
        offset: u64,
        /// Encoding.
        encoding: NativeModScalar,
        /// Item identifier.
        item: ItemId,
    },
    /// Inventory capacity binding.
    InventoryCapacity {
        /// Field offset.
        offset: u64,
        /// Encoding.
        encoding: NativeModScalar,
        /// Item identifier.
        item: ItemId,
    },
    /// Constant binding.
    Constant {
        /// Field offset.
        offset: u64,
        /// Encoding.
        encoding: NativeModScalar,
        /// Value.
        value: f64,
    },
}

/// Native shared actor field.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeModSharedActorField {
    /// Field offset.
    pub offset: u64,
    /// Access mode (the native reader leaves this unset).
    pub access: Option<QvmModFieldAccess>,
    /// Field binding.
    pub binding: NativeModSharedActorBinding,
}

/// Native shared actor binding.
#[derive(Debug, Clone, PartialEq)]
pub enum NativeModSharedActorBinding {
    /// Origin.
    Origin,
    /// Velocity.
    Velocity,
    /// Angles.
    Angles,
    /// Bounds minimum.
    BoundsMin,
    /// Bounds maximum.
    BoundsMax,
    /// Record link.
    Record {
        /// Record name.
        record: String,
    },
    /// Constant vector.
    ConstantVector(Vec3),
    /// Private storage.
    Private {
        /// Byte length.
        byte_length: u64,
    },
}

/// Native mod actor record.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeModActorRecord {
    /// Record identity.
    pub id: String,
    /// Record base.
    pub base: NativeModRecordBase,
    /// Record stride.
    pub stride: u64,
    /// First slot.
    pub first_slot: u64,
    /// Capacity.
    pub capacity: u64,
    /// Record fields.
    pub fields: Vec<NativeModActorField>,
}

/// Native mod actor record base.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NativeModRecordBase {
    /// Entities base.
    Entities,
    /// Clients base.
    Clients,
    /// Address base.
    Address(NativeModAddress),
}

/// Native mod source actors.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeModSourceActors {
    /// Allocate entry.
    pub allocate: NativeModEntry,
    /// Release entry.
    pub release: NativeModEntry,
    /// Update entry.
    pub update: NativeModUpdate,
    /// Frame seconds.
    pub frame_seconds: f64,
    /// Clock bindings.
    pub clock: Vec<NativeModClock>,
    /// Field offsets.
    pub fields: NativeModSourceActorFields,
    /// Callbacks.
    pub callbacks: Option<NativeModSourceActorCallbacks>,
    /// Combat.
    pub combat: Option<NativeModCombat>,
}

/// Native mod update entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NativeModUpdate {
    /// Entry.
    pub entry: NativeModEntry,
    /// Return encoding.
    pub returns: NativeModReturn,
}

/// Native mod clock binding.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NativeModClock {
    /// Clock address.
    pub address: NativeModAddress,
    /// Clock input.
    pub input: NativeModClockInput,
    /// Encoding.
    pub encoding: NativeModScalar,
    /// Units.
    pub units: ModTimeUnits,
}

/// Native mod clock input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeModClockInput {
    /// Time.
    Time,
    /// Frame.
    Frame,
}

/// Native mod source actor field offsets.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeModSourceActorFields {
    /// Velocity offset.
    pub velocity: u64,
    /// Ground offset.
    pub ground: u64,
    /// Use offset.
    pub r#use: Option<u64>,
    /// Think offset.
    pub think: u64,
    /// Nextthink.
    pub nextthink: NativeModNextthink,
}

/// Native mod nextthink.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeModNextthink {
    /// Field offset.
    pub offset: u64,
    /// Encoding.
    pub encoding: NativeModScalar,
    /// Units.
    pub units: ModTimeUnits,
}

/// Native mod source actor callbacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeModSourceActorCallbacks {
    /// Callback ABI.
    pub abi: Q2CallbackAbi,
    /// Touch offset.
    pub touch: Option<u64>,
    /// Pain offset.
    pub pain: Option<u64>,
    /// Die offset.
    pub die: Option<u64>,
}

/// Quake II callback ABI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2CallbackAbi {
    /// Classic ABI.
    Classic,
    /// Rerelease ABI.
    Rerelease,
}

/// Native mod armor selection.
#[derive(Debug, Clone, PartialEq)]
pub enum NativeModArmorSelection {
    /// Positive selection.
    Positive {
        /// Selection field.
        field: NativeModArmorField,
    },
    /// Enum selection.
    Enum {
        /// Selection field.
        field: NativeModArmorField,
        /// Selected value.
        value: f64,
        /// None value.
        none: f64,
    },
}

/// Native mod power armor item.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeModPowerArmorItem {
    /// Item identifier.
    pub item: ItemId,
    /// Protection kind.
    pub kind: NativePowerArmorKind,
    /// Selection.
    pub selection: NativeModArmorSelection,
    /// Cells field.
    pub cells: NativeModArmorField,
    /// Enabled test.
    pub enabled: Option<NativeModMaskedField>,
}

/// Native power armor kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativePowerArmorKind {
    /// Screen.
    Screen,
    /// Shield.
    Shield,
}

/// Native mod regular armor item.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeModRegularArmorItem {
    /// Item identifier.
    pub item: Option<ItemId>,
    /// Selection.
    pub selection: NativeModArmorSelection,
    /// Points field.
    pub points: NativeModArmorField,
}

/// Native mod armor.
#[derive(Debug, Clone, PartialEq)]
pub enum NativeModArmor {
    /// No armor.
    None,
    /// Quake II armor.
    Q2 {
        /// Regular armor.
        regular: Vec<NativeModQ2RegularArmor>,
        /// Power armor.
        power: Vec<NativeModPowerArmorItem>,
    },
    /// Source armor.
    Source {
        /// Regular armor.
        regular: Vec<NativeModRegularArmorItem>,
        /// Power armor.
        power: Vec<NativeModPowerArmorItem>,
    },
}

/// Native Quake II regular armor entry.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeModQ2RegularArmor {
    /// Item identifier.
    pub item: ItemId,
    /// Selection.
    pub selection: NativeModArmorSelection,
    /// Points field.
    pub points: NativeModArmorField,
    /// Normal protection.
    pub normal_protection: f64,
    /// Energy protection.
    pub energy_protection: f64,
}

/// Native mod protection definition.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeModProtectionDefinition {
    /// Protection identity.
    pub id: String,
    /// Admission.
    pub admission: Option<ModProtectionAdmission>,
    /// Channel.
    pub channel: NativeModProtectionChannel,
}

/// Native mod protection channel.
#[derive(Debug, Clone, PartialEq)]
pub enum NativeModProtectionChannel {
    /// Powered channel.
    Powered {
        /// Storage.
        storage: Vec<NativeModPowerArmorItem>,
        /// Absorption.
        absorb: NativeModProtectionAbsorb,
    },
    /// Regular channel.
    Regular {
        /// Storage.
        storage: Vec<NativeModRegularArmorItem>,
        /// Absorption.
        absorb: NativeModRegularAbsorb,
    },
}

/// Native powered protection absorption.
#[derive(Debug, Clone, PartialEq)]
pub enum NativeModProtectionAbsorb {
    /// Source call absorption.
    SourceCall(NativeModSourceCall),
    /// Region absorption.
    Region(NativeModProtectionRegion),
    /// Quake II power armor call.
    Q2PowerArmor(NativeModQ2ArmorCall),
}

/// Native regular protection absorption.
#[derive(Debug, Clone, PartialEq)]
pub enum NativeModRegularAbsorb {
    /// Source call absorption.
    SourceCall(NativeModSourceCall),
    /// Region absorption.
    Region(NativeModProtectionRegion),
    /// Quake II armor call.
    Q2Armor(NativeModQ2ArmorCheck),
}

/// Native Quake II armor call.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeModQ2ArmorCall {
    /// Call entry.
    pub entry: NativeModEntry,
    /// Flag ABI.
    pub flags: Q2CallbackAbi,
    /// Globals.
    pub globals: Vec<NativeModGlobal>,
}

/// Native Quake II armor check call.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeModQ2ArmorCheck {
    /// Call entry.
    pub entry: NativeModEntry,
    /// Flag ABI.
    pub flags: Q2CallbackAbi,
    /// Globals.
    pub globals: Vec<NativeModGlobal>,
    /// Sparks argument.
    pub sparks: u32,
}

/// Native mod deferred damage.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NativeModDeferredDamage {
    /// Process entry.
    pub process: NativeModEntry,
    /// Attacker offset.
    pub attacker: u64,
    /// Inflictor offset.
    pub inflictor: u64,
    /// Blood field.
    pub blood: NativeModScalarField,
    /// Knockback field.
    pub knockback: NativeModScalarField,
    /// Point offset.
    pub point: u64,
    /// Mod offset.
    pub r#mod: u64,
    /// Receipt offset.
    pub receipt: u64,
}

/// Native mod combat.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeModCombat {
    /// Damage entry.
    pub damage: NativeModDamageEntry,
    /// Damage causes.
    pub causes: NativeModDamageCauses,
    /// Health field.
    pub health: NativeModScalarField,
    /// Mass field.
    pub mass: NativeModScalarField,
    /// Take-damage field.
    pub takedamage: NativeModScalarField,
    /// Flag field.
    pub flags: NativeModCombatFlags,
    /// Armor.
    pub armor: NativeModArmor,
    /// Deferred damage.
    pub deferred: Option<NativeModDeferredDamage>,
}

/// Native mod damage entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NativeModDamageEntry {
    /// Entry.
    pub entry: NativeModEntry,
    /// Entry ABI.
    pub abi: Q2CallbackAbi,
}

/// Native mod damage causes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeModDamageCauses {
    /// Classic causes.
    Classic {
        /// Game.
        game: NativeClassicGame,
    },
    /// Rerelease causes.
    Rerelease,
}

/// Native classic game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeClassicGame {
    /// Base game.
    Base,
    /// Xatrix.
    Xatrix,
    /// Rogue.
    Rogue,
    /// Capture the flag.
    Ctf,
}

/// Native mod combat flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeModCombatFlags {
    /// Field offset.
    pub offset: u64,
    /// Encoding.
    pub encoding: NativeModScalar,
    /// Invulnerable bit.
    pub invulnerable: u64,
    /// No-knockback bit.
    pub no_knockback: u64,
}

/// Native mod declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeModDeclaration {
    /// Objective declarations.
    pub objectives: Vec<SourceObjectiveDeclaration<NativeModObjectiveStorage, NativeModAddress, NativeModSourceCall>>,
    /// Client presentation.
    pub client_presentation: Option<NativeModClientPresentation>,
    /// Schema version (1).
    pub version: u32,
    /// Program artifact.
    pub program: ModProgram,
    /// Target API plus ABI.
    pub target: NativeModTarget,
    /// Source actors.
    pub source_actors: Option<NativeModSourceActors>,
    /// Protection declarations.
    pub protection: Vec<NativeModProtectionDefinition>,
    /// Pickup rules.
    pub pickups: Vec<NativeModPickup>,
    /// Item declarations.
    pub items: Option<NativeModItems>,
    /// Client bindings.
    pub clients: Option<NativeModClients>,
    /// Console variables.
    pub cvars: Vec<ModCvar>,
    /// Spawn entities function.
    pub spawn_entities: Option<String>,
    /// Actor records.
    pub actor_records: Vec<NativeModActorRecord>,
    /// Entity record.
    pub entity_record: Option<String>,
    /// Initialize calls.
    pub initialize: Vec<NativeModSourceCall>,
    /// Project calls.
    pub project: Vec<NativeModSourceCall>,
    /// Release calls.
    pub release: Vec<NativeModSourceCall>,
    /// Callbacks.
    pub callbacks: Vec<NativeModCallback>,
}

/// Native mod objective storage.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NativeModObjectiveStorage {
    /// Storage address.
    pub address: NativeModAddress,
    /// Encoding.
    pub encoding: NativeModScalar,
}

/// Native mod target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeModTarget {
    /// Classic game on Windows i386.
    ClassicWindowsI386,
    /// Rerelease game on Windows x86-64.
    RereleaseWindowsX86_64,
}

// `native-mod-items.ts`.

/// Native item field.
pub type NativeItemField = NativeModArmorField;

/// Native item pointer.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NativeItemPointer {
    /// Record name.
    pub record: String,
    /// Field offset.
    pub offset: u64,
}

/// Native item capacity.
#[derive(Debug, Clone, PartialEq)]
pub enum NativeItemCapacity {
    /// Constant capacity.
    Constant {
        /// Value.
        value: f64,
    },
    /// Field capacity.
    Field {
        /// Field.
        field: NativeItemField,
    },
    /// Source capacity.
    Source {
        /// Source address.
        address: NativeModAddress,
        /// Encoding.
        encoding: NativeModScalar,
    },
}

/// Native item storage.
#[derive(Debug, Clone, PartialEq)]
pub enum NativeItemStorage {
    /// Counter storage.
    Counter {
        /// Storage field.
        field: NativeItemField,
        /// Item identifier.
        item: ItemId,
        /// Capacity.
        capacity: NativeItemCapacity,
    },
    /// Bit storage.
    Bits {
        /// Storage field.
        field: NativeItemField,
        /// Private mask.
        private_mask: f64,
        /// Masked items.
        items: Vec<NativeMaskedItem>,
    },
}

/// Masked native storage item.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeMaskedItem {
    /// Item identifier.
    pub item: ItemId,
    /// Mask.
    pub mask: f64,
}

/// Native item test.
#[derive(Debug, Clone, PartialEq)]
pub enum NativeItemTest {
    /// Scalar test.
    Scalar {
        /// Tested field.
        field: NativeItemField,
        /// Test mask.
        mask: Option<f64>,
        /// Comparison.
        comparison: ItemTestComparison,
        /// Value.
        value: f64,
    },
    /// Pointer test.
    Pointer {
        /// Tested field.
        field: NativeItemPointer,
        /// Expected address.
        value: Option<NativeModAddress>,
    },
}

/// Native weapon stage.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeWeaponStage {
    /// Dispatcher.
    pub dispatcher: NativeWeaponDispatcher,
    /// Input-read decisions.
    pub decisions: Vec<NativeWeaponDecision>,
    /// Committed input tests.
    pub committed_input: Vec<Vec<NativeItemTest>>,
    /// Continuation tests.
    pub continuations: Vec<Vec<NativeItemTest>>,
    /// Settled tests.
    pub settled: Vec<Vec<NativeItemTest>>,
    /// Selection.
    pub selection: NativeWeaponSelection,
}

/// Native weapon dispatcher.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NativeWeaponDispatcher {
    /// Entry.
    pub entry: NativeModEntry,
    /// Record name.
    pub record: String,
    /// Argument offset.
    pub argument: u64,
    /// Argument count.
    pub arguments: u64,
}

/// Native weapon decision region.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NativeWeaponDecision {
    /// Entry offset.
    pub entry: u64,
    /// Join offset.
    pub join: u64,
    /// Cleared fields.
    pub fields: Vec<NativeWeaponClearedField>,
}

/// Native weapon cleared field.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NativeWeaponClearedField {
    /// Cleared field.
    pub field: NativeModArmorField,
    /// Clear mask.
    pub clear_mask: u64,
}

/// Native weapon selection.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeWeaponSelection {
    /// Active weapon pointer.
    pub active: NativeItemPointer,
    /// Pending weapon pointer.
    pub pending: Option<NativeItemPointer>,
    /// Selection values.
    pub values: Vec<NativeWeaponValue>,
}

/// Native weapon value.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeWeaponValue {
    /// Weapon item.
    pub item: ItemId,
    /// Weapon address.
    pub address: NativeModAddress,
    /// Request call.
    pub request: NativeModSourceCall,
}

/// Native mod items.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeModItems {
    /// Item definitions.
    pub definitions: Vec<NativeItemDefinition>,
    /// Item storage.
    pub storage: Vec<NativeItemStorage>,
    /// Weapon stage.
    pub weapons: Option<NativeWeaponStage>,
}

/// Native item definition.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeItemDefinition {
    /// Item identifier.
    pub item: ItemId,
    /// Display label.
    pub label: String,
    /// Item icon.
    pub icon: Option<SourceItemIconDeclaration>,
    /// Admission mode.
    pub admission: SourceItemAdmissionMode,
    /// Action calls.
    pub actions: Option<SourceItemActionCalls<NativeModSourceCall>>,
    /// Item kind.
    pub kind: NativeItemKind,
}

/// Native item kind.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum NativeItemKind {
    /// Counter item.
    Counter,
    /// Weapon item.
    Weapon(SourceWeaponItem),
}

// `native-mod-region.ts`.

/// Native region register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeModRegionRegister {
    /// RAX.
    Rax,
    /// RCX.
    Rcx,
    /// RDX.
    Rdx,
    /// RBX.
    Rbx,
    /// RBP.
    Rbp,
    /// RSI.
    Rsi,
    /// RDI.
    Rdi,
    /// R8.
    R8,
    /// R9.
    R9,
    /// R10.
    R10,
    /// R11.
    R11,
    /// R12.
    R12,
    /// R13.
    R13,
    /// R14.
    R14,
    /// R15.
    R15,
}

/// Native region storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeModRegionStorage {
    /// Scalar storage.
    Scalar(NativeModScalar),
    /// Pointer storage.
    Pointer,
}

/// Native region location.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeModRegionLocation {
    /// Register location.
    Register {
        /// Register.
        register: NativeModRegionRegister,
        /// Storage.
        storage: NativeModRegionStorage,
    },
    /// SIMD location.
    Simd {
        /// Register index.
        index: u32,
        /// Byte offset.
        offset: u64,
        /// Storage.
        storage: NativeModScalar,
    },
    /// Stack location.
    Stack {
        /// Byte offset.
        offset: u64,
        /// Storage.
        storage: NativeModRegionStorage,
    },
}

/// Native mod protection region.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeModProtectionRegion {
    /// Region call.
    pub call: NativeModSourceCall,
    /// Stack frame.
    pub frame: NativeModRegionFrame,
    /// Entry offset.
    pub entry: u64,
    /// Join offset.
    pub join: u64,
    /// Region inputs.
    pub inputs: Vec<NativeModRegionInput>,
    /// Result location.
    pub result: NativeModRegionLocation,
}

/// Native mod region stack frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeModRegionFrame {
    /// Entry offset.
    pub entry: u64,
    /// Exit offset.
    pub exit: u64,
    /// Stack bytes.
    pub stack_bytes: u64,
    /// Argument bytes.
    pub argument_bytes: u64,
}

/// Native mod region input.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeModRegionInput {
    /// Target location.
    pub target: NativeModRegionLocation,
    /// Value.
    pub value: NativeModValue,
}

// Boundary guest checkpoints (donor `execution.ts`; `GuestCheckpoint`
// plus the shapes it closes over).

/// Guest address (donor `execution.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GuestAddress {
    /// Address space token.
    pub address_space: u64,
    /// Byte offset.
    pub byte_offset: u64,
}

/// Guest storage encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GuestStorage {
    /// Signed 8-bit.
    Int8,
    /// Unsigned 8-bit.
    Uint8,
    /// Signed 16-bit.
    Int16,
    /// Unsigned 16-bit.
    Uint16,
    /// Signed 32-bit.
    Int32,
    /// Unsigned 32-bit.
    Uint32,
    /// Signed 64-bit.
    Int64,
    /// Unsigned 64-bit.
    Uint64,
    /// 32-bit float.
    Float32,
    /// 64-bit float.
    Float64,
    /// Pointer.
    Pointer,
}

/// Guest field layout.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GuestFieldLayout {
    /// Field name.
    pub name: String,
    /// Byte offset.
    pub byte_offset: u64,
    /// Storage.
    pub storage: GuestStorage,
    /// Element count.
    pub count: u64,
}

/// Guest aggregate layout.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GuestLayout {
    /// Layout identity.
    pub id: String,
    /// Byte length.
    pub byte_length: u64,
    /// Alignment.
    pub alignment: u64,
    /// Pointer bytes.
    pub pointer_bytes: u8,
    /// Field layouts.
    pub fields: Vec<GuestFieldLayout>,
}

/// Guest private state.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GuestPrivateState {
    /// Owning module.
    pub module: ModuleIdentity,
    /// State format.
    pub format: String,
    /// State bytes.
    pub bytes: Vec<u8>,
}

/// Guest memory region.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GuestMemoryRegion {
    /// Base address.
    pub base: u64,
    /// Permissions.
    pub permissions: GuestMemoryPermissions,
    /// Region bytes.
    pub bytes: Vec<u8>,
}

/// Guest memory permissions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GuestMemoryPermissions {
    /// Read.
    Read,
    /// Read-write.
    ReadWrite,
    /// Read-execute.
    ReadExecute,
    /// Read-write-execute.
    ReadWriteExecute,
}

/// Random generator state (donor `numeric.ts`).
#[derive(Debug, Clone, PartialEq)]
pub enum RandomState {
    /// Quake III LCG.
    Q3Lcg {
        /// Seed.
        seed: f64,
        /// Draws.
        draws: f64,
    },
    /// MSVCRT rand.
    MsvcrtRand {
        /// Seed.
        seed: f64,
        /// Draws.
        draws: f64,
    },
    /// glibc random.
    GlibcRandom {
        /// State words.
        words: Vec<f64>,
        /// Front index.
        front: f64,
        /// Rear index.
        rear: f64,
        /// Draws.
        draws: f64,
    },
    /// Quake II rerelease MT19937.
    Q2RereleaseMt19937 {
        /// State words.
        words: Vec<f64>,
        /// Index.
        index: f64,
        /// Draws.
        draws: f64,
    },
    /// Guest generator.
    Guest {
        /// Module name.
        module: String,
        /// State bytes.
        bytes: Vec<u8>,
        /// Draws.
        draws: f64,
    },
}

/// QuakeC stack frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QuakeCStackFrame {
    /// Statement index.
    pub statement: u32,
    /// Function index.
    pub function_index: u32,
}

/// Guest checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub enum GuestCheckpoint {
    /// QuakeC checkpoint.
    Quakec(QuakeCCheckpoint),
    /// QVM checkpoint.
    Qvm(QvmCheckpoint),
    /// Native guest checkpoint.
    NativeGuest(NativeGuestCheckpoint),
    /// TypeScript guest checkpoint.
    Typescript(TypeScriptGuestCheckpoint),
}

/// QuakeC checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct QuakeCCheckpoint {
    /// Owning module.
    pub module: ModuleIdentity,
    /// Random states.
    pub random: Vec<RandomState>,
    /// API identity.
    pub api: QuakeCApiIdentity,
    /// Global bytes.
    pub globals: Vec<u8>,
    /// Entity bytes.
    pub entities: Vec<u8>,
    /// Entity stride bytes.
    pub entity_stride_bytes: u64,
    /// Entity count.
    pub entity_count: u64,
    /// String bytes.
    pub strings: Vec<u8>,
    /// Statement index.
    pub statement: u32,
    /// Function index.
    pub function_index: u32,
    /// Argument count.
    pub argument_count: u64,
    /// Call stack.
    pub call_stack: Vec<QuakeCStackFrame>,
    /// Locals bytes.
    pub locals: Vec<u8>,
    /// Host state.
    pub host_state: GuestPrivateState,
}

/// QVM checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmCheckpoint {
    /// Owning module.
    pub module: ModuleIdentity,
    /// Random states.
    pub random: Vec<RandomState>,
    /// ABI profile.
    pub abi_profile: Option<QvmAbiProfile>,
    /// API identity.
    pub api: Q3ApiIdentity,
    /// Data bytes.
    pub data: Vec<u8>,
    /// Instruction index.
    pub instruction_index: u32,
    /// Program stack.
    pub program_stack: u32,
    /// Operand stack.
    pub operand_stack: Vec<f64>,
    /// Host state.
    pub host_state: GuestPrivateState,
}

/// Native guest checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeGuestCheckpoint {
    /// Owning module.
    pub module: ModuleIdentity,
    /// Random states.
    pub random: Vec<RandomState>,
    /// ABI.
    pub abi: NativeAbi,
    /// Memory regions.
    pub regions: Vec<GuestMemoryRegion>,
    /// Processor layout.
    pub processor_layout: GuestLayout,
    /// Processor state bytes.
    pub processor_state: Vec<u8>,
    /// Runtime state.
    pub runtime_state: GuestPrivateState,
}

/// TypeScript guest checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeScriptGuestCheckpoint {
    /// Owning module.
    pub module: ModuleIdentity,
    /// Random states.
    pub random: Vec<RandomState>,
    /// API identity.
    pub api: GameApiIdentity,
    /// Guest state.
    pub state: GuestPrivateState,
}

/// Game API identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GameApiIdentity {
    /// QuakeC API.
    Quakec(QuakeCApiIdentity),
    /// Quake II game API.
    Q2Game(Q2GameApiIdentity),
    /// Quake II client game API.
    Q2Cgame,
    /// Quake III API.
    Q3(Q3ApiIdentity),
}

// Boundary weapon behavior (donor `weapon-behavior.ts`; referenced
// shapes plus the `same*` comparisons `content.ts` closes over).

/// Guest callback reference (donor `execution.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GuestCallbackReference {
    /// QuakeC function.
    Quakec {
        /// Owning module.
        module: ModuleIdentity,
        /// Function index.
        function_index: u32,
    },
    /// QVM instruction.
    Qvm {
        /// Owning module.
        module: ModuleIdentity,
        /// Instruction index.
        instruction_index: u32,
    },
    /// Native guest address.
    NativeGuest {
        /// Owning module.
        module: ModuleIdentity,
        /// Callback address.
        address: GuestAddress,
        /// Call ABI.
        abi: NativeCallAbi,
    },
}

/// Projectile role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProjectileRole {
    /// Rocket.
    Rocket,
    /// Grenade.
    Grenade,
    /// Nail.
    Nail,
    /// Bolt.
    Bolt,
    /// Plasma.
    Plasma,
    /// Energy.
    Energy,
    /// Grapple.
    Grapple,
}

/// Weapon behavior callback.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WeaponBehaviorCallback {
    /// QuakeC function.
    Quakec {
        /// Owning module.
        module: ModuleIdentity,
        /// Function index.
        function_index: u32,
    },
    /// QVM instruction.
    Qvm {
        /// Owning module.
        module: ModuleIdentity,
        /// Instruction index.
        instruction_index: u32,
    },
    /// Native artifact offset.
    NativeArtifact {
        /// Owning module.
        module: ModuleIdentity,
        /// Image offset.
        image_offset: u64,
        /// Call ABI.
        abi: NativeCallAbi,
    },
}

/// Weapon behavior definition.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WeaponBehaviorDefinition {
    /// Definition identity.
    pub id: String,
    /// Title.
    pub title: String,
    /// Owning module.
    pub module: ModuleIdentity,
    /// Projectile role.
    pub role: ProjectileRole,
    /// Activate callback.
    pub activate: Option<WeaponBehaviorCallback>,
    /// Fire callback.
    pub fire: WeaponBehaviorCallback,
}

fn same_module(left: &ModuleIdentity, right: &ModuleIdentity) -> bool {
    left.id == right.id
        && left.digest == right.digest
        && left.artifact_path == right.artifact_path
        && left.revision == right.revision
}

fn same_callback(left: Option<&WeaponBehaviorCallback>, right: Option<&WeaponBehaviorCallback>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(_), None) | (None, Some(_)) => false,
        (Some(left), Some(right)) => {
            let (left_module, right_module) = match (left, right) {
                (
                    WeaponBehaviorCallback::Quakec { module: left, .. }
                    | WeaponBehaviorCallback::Qvm { module: left, .. }
                    | WeaponBehaviorCallback::NativeArtifact { module: left, .. },
                    WeaponBehaviorCallback::Quakec { module: right, .. }
                    | WeaponBehaviorCallback::Qvm { module: right, .. }
                    | WeaponBehaviorCallback::NativeArtifact { module: right, .. },
                ) => (left, right),
            };
            if !same_module(left_module, right_module) {
                return false;
            }
            match (left, right) {
                (
                    WeaponBehaviorCallback::Quakec {
                        function_index: left, ..
                    },
                    WeaponBehaviorCallback::Quakec {
                        function_index: right, ..
                    },
                ) => left == right,
                (
                    WeaponBehaviorCallback::Qvm {
                        instruction_index: left,
                        ..
                    },
                    WeaponBehaviorCallback::Qvm {
                        instruction_index: right,
                        ..
                    },
                ) => left == right,
                (
                    WeaponBehaviorCallback::NativeArtifact {
                        image_offset: left_offset,
                        abi: left_abi,
                        ..
                    },
                    WeaponBehaviorCallback::NativeArtifact {
                        image_offset: right_offset,
                        abi: right_abi,
                        ..
                    },
                ) => left_offset == right_offset && left_abi == right_abi,
                _ => false,
            }
        }
    }
}

/// Compare weapon behavior definitions.
#[must_use]
pub fn same_weapon_behavior(left: &WeaponBehaviorDefinition, right: &WeaponBehaviorDefinition) -> bool {
    left.id == right.id
        && left.role == right.role
        && same_module(&left.module, &right.module)
        && same_callback(Some(&left.fire), Some(&right.fire))
        && same_callback(left.activate.as_ref(), right.activate.as_ref())
}

/// Author-declared QVM weapon behavior layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QvmWeaponBehaviorLayout {
    /// Entity stride.
    pub entity_stride: u64,
    /// Level time address.
    pub level_time: u32,
    /// Allocate entry.
    pub allocate: u32,
    /// Free entry.
    pub free: u32,
    /// Behavior fields.
    pub fields: QvmWeaponBehaviorFields,
}

/// QVM weapon behavior fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QvmWeaponBehaviorFields {
    /// In-use offset.
    pub inuse: u32,
    /// Nextthink offset.
    pub nextthink: u32,
    /// Think offset.
    pub think: u32,
    /// Health offset.
    pub health: u32,
}

/// Compare QVM weapon behavior layouts.
#[must_use]
pub fn same_qvm_weapon_layout(left: &QvmWeaponBehaviorLayout, right: &QvmWeaponBehaviorLayout) -> bool {
    left == right
}

// Boundary native weapon behavior (donor `native-weapon-behavior.ts`).

/// Native weapon registration layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeWeaponRegistrationLayout {
    /// Byte length.
    pub byte_length: u64,
    /// Name offset.
    pub name: u64,
    /// Tag offset.
    pub tag: u64,
    /// Callback offset.
    pub callback: u64,
}

/// Native weapon entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NativeWeaponEntry {
    /// Relative virtual address.
    pub rva: u64,
    /// Registration.
    pub registration: Option<NativeWeaponRegistration>,
}

/// Native weapon registration.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NativeWeaponRegistration {
    /// Relative virtual address.
    pub rva: u64,
    /// Registration name.
    pub name: String,
    /// Tag.
    pub tag: u64,
    /// Layout.
    pub layout: NativeWeaponRegistrationLayout,
}

/// Native weapon command.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NativeWeaponCommand {
    /// Arguments.
    pub arguments: Vec<String>,
    /// Tail.
    pub tail: String,
}

/// Native weapon console variable.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NativeWeaponCvar {
    /// Variable name.
    pub name: String,
    /// Variable value.
    pub value: String,
}

/// Native weapon behavior declaration.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NativeWeaponBehaviorDeclaration {
    /// Schema version (1).
    pub version: u32,
    /// Definition identity.
    pub id: String,
    /// Title.
    pub title: String,
    /// Projectile role.
    pub role: ProjectileRole,
    /// Artifact path.
    pub artifact_path: String,
    /// Artifact digest.
    pub artifact_digest: ContentDigest,
    /// Entity layout.
    pub entity: NativeWeaponEntity,
    /// Client layout.
    pub client: NativeWeaponClient,
    /// Equipped weapon layout.
    pub equipped_weapon: NativeWeaponEquipped,
    /// Time storage.
    pub time: NativeWeaponTime,
    /// Think entry.
    pub think: NativeWeaponThink,
    /// Allocate entry.
    pub allocate: NativeWeaponAllocate,
    /// Free entry.
    pub free: NativeWeaponFree,
    /// Projectile touch entry.
    pub projectile_touch: NativeWeaponEntry,
    /// Equip entries.
    pub equip: NativeWeaponCalls,
    /// Launch entries.
    pub launch: NativeWeaponCalls,
    /// Activate RVA.
    pub activate_rva: Option<u64>,
    /// Fire RVA.
    pub fire_rva: u64,
    /// Initialization classes.
    pub initialization_classes: Vec<String>,
    /// Equipment commands.
    pub equipment: Vec<NativeWeaponCommand>,
    /// Ammunition command.
    pub ammunition: NativeWeaponCommand,
    /// Initial console variables.
    pub initial_cvars: Vec<NativeWeaponCvar>,
    /// Provisioning console variables.
    pub provisioning_cvars: Vec<NativeWeaponCvar>,
}

/// Native weapon entity layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeWeaponEntity {
    /// Byte length.
    pub byte_length: u64,
    /// Origin offset.
    pub origin: u64,
    /// Angles offset.
    pub angles: u64,
    /// Velocity offset.
    pub velocity: u64,
    /// Client offset.
    pub client: u64,
    /// Owner offset.
    pub owner: u64,
    /// View height offset.
    pub view_height: u64,
    /// Generation offset.
    pub generation: u64,
    /// Next think offset.
    pub next_think: u64,
    /// Think callback offset.
    pub think_callback: u64,
    /// Think registration offset.
    pub think_registration: u64,
    /// Touch callback offset.
    pub touch_callback: u64,
}

/// Native weapon client layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeWeaponClient {
    /// Byte length.
    pub byte_length: u64,
    /// Weapon offset.
    pub weapon: u64,
    /// View angles offset.
    pub view_angles: u64,
    /// Forward offset.
    pub forward: u64,
}

/// Native equipped weapon layout.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NativeWeaponEquipped {
    /// Byte length.
    pub byte_length: u64,
    /// Callback offset.
    pub callback: u64,
    /// Expected entry.
    pub expected: NativeWeaponEntry,
}

/// Native weapon time storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeWeaponTime {
    /// Time RVA (int64 milliseconds).
    pub rva: u64,
}

/// Native weapon think entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeWeaponThink {
    /// Think tag.
    pub tag: u64,
    /// Registration layout.
    pub registration: NativeWeaponRegistrationLayout,
}

/// Native weapon allocate entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NativeWeaponAllocate {
    /// Allocate entry.
    pub entry: NativeWeaponEntry,
}

/// Native weapon free entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NativeWeaponFree {
    /// Free entry.
    pub entry: NativeWeaponEntry,
}

/// Native weapon call list.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NativeWeaponCalls {
    /// Calls.
    pub calls: Vec<NativeWeaponEntry>,
}

// Boundary grapple (donor `qvm-grapple.ts`).

/// QVM grapple definition.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmGrappleDefinition {
    /// Definition identity.
    pub id: String,
    /// Title.
    pub title: String,
    /// Owning module.
    pub module: ModuleIdentity,
    /// ABI profile.
    pub abi_profile: QvmAbiProfile,
    /// Entity stride.
    pub entity_stride: u64,
    /// Client stride.
    pub client_stride: u64,
    /// Grapple fields.
    pub fields: QvmGrappleFields,
    /// Grapple globals.
    pub globals: QvmGrappleGlobals,
    /// Grapple callbacks.
    pub callbacks: QvmGrappleCallbacks,
    /// Fire arguments.
    pub fire_arguments: Vec<f64>,
    /// Movement words.
    pub movement: QvmGrappleMovement,
    /// Initial console variables.
    pub initial_cvars: HashMap<String, String>,
    /// Event lifetime milliseconds.
    pub event_lifetime_milliseconds: f64,
    /// Grapple damage method.
    pub grapple_damage_method: f64,
    /// Presentation.
    pub presentation: QvmGrapplePresentation,
    /// Pulling flag.
    pub pulling_flag: f64,
}

/// QVM grapple fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QvmGrappleFields {
    /// In-use offset.
    pub inuse: u32,
    /// Client offset.
    pub client: u32,
    /// Parent offset.
    pub parent: u32,
    /// Target offset.
    pub target: u32,
    /// Mover offset.
    pub mover: Option<u32>,
    /// Hook offset.
    pub hook: u32,
    /// Health offset.
    pub health: u32,
    /// Take-damage offset.
    pub takedamage: u32,
    /// Event time offset.
    pub event_time: u32,
    /// Free-after-event offset.
    pub free_after_event: u32,
}

/// QVM grapple globals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QvmGrappleGlobals {
    /// Time address.
    pub time: u32,
    /// Frame address.
    pub frame: u32,
    /// Movement address.
    pub movement: u32,
    /// Forward address.
    pub forward: u32,
    /// Ground plane address.
    pub ground_plane: u32,
}

/// QVM grapple callbacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QvmGrappleCallbacks {
    /// Allocate entry.
    pub allocate: u32,
    /// Free entry.
    pub free: u32,
    /// Fire entry.
    pub fire: u32,
    /// Release entry.
    pub release: u32,
    /// Force release entry.
    pub force_release: u32,
    /// Missile entry.
    pub missile: u32,
    /// Follow entry.
    pub follow: Option<u32>,
    /// Think entry.
    pub think: u32,
    /// Pull entry.
    pub pull: u32,
    /// Move mover hooks entry.
    pub move_mover_hooks: Option<u32>,
    /// Damage entry.
    pub damage: u32,
    /// Same team entry.
    pub same_team: u32,
    /// Player move entry.
    pub player_move: u32,
}

/// QVM grapple movement words.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QvmGrappleMovement {
    /// Byte length.
    pub byte_length: u64,
    /// Movement words.
    pub words: Vec<QvmGrappleWord>,
}

/// QVM grapple movement word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QvmGrappleWord {
    /// Word offset.
    pub offset: u32,
    /// Word value.
    pub value: u32,
}

/// QVM grapple presentation.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmGrapplePresentation {
    /// Projectile model.
    pub projectile_model: String,
    /// View model.
    pub view_model: String,
    /// Weapon index.
    pub weapon_index: f64,
    /// View anchor.
    pub view_anchor: QvmGrappleViewAnchor,
    /// View attachments.
    pub view_attachments: Vec<QvmGrappleViewAttachment>,
    /// Cable.
    pub cable: QvmGrappleCable,
    /// Fire sound.
    pub fire_sound: Option<String>,
    /// Attach sound.
    pub attach_sound: Option<String>,
    /// Release sound.
    pub release_sound: Option<String>,
    /// Pull sound.
    pub pull_sound: Option<String>,
    /// Hang sound.
    pub hang_sound: Option<String>,
}

/// QVM grapple view anchor.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmGrappleViewAnchor {
    /// Anchor path.
    pub path: String,
    /// Anchor tag.
    pub tag: String,
    /// Anchor offset.
    pub offset: Vec3,
    /// Field-of-view offset.
    pub fov_offset: QvmGrappleFovOffset,
}

/// QVM grapple field-of-view offset.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QvmGrappleFovOffset {
    /// Above.
    pub above: f64,
    /// Scale.
    pub scale: f64,
}

/// QVM grapple view attachment.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QvmGrappleViewAttachment {
    /// Attachment path.
    pub path: String,
    /// Attachment tag.
    pub tag: String,
}

/// QVM grapple cable.
#[derive(Debug, Clone, PartialEq)]
pub enum QvmGrappleCable {
    /// Shader cable.
    Shader {
        /// Shader path.
        path: String,
        /// Width.
        width: f64,
    },
    /// Model cable.
    Model {
        /// Flight model.
        flight: String,
        /// Pull model.
        pull: String,
        /// Hold model.
        hold: String,
        /// Segment length.
        segment_length: f64,
    },
}

// Component presentation ownership (donor `presentation.ts`).

/// A component activation, retained with its output across a world checkpoint.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PresentationOwner {
    /// Owning provider.
    pub provider: ProviderId,
    /// Activation generation.
    pub generation: u64,
}

/// Media request published by a component presentation.
#[derive(Debug, Clone, PartialEq)]
pub enum ComponentPresentationMediaRequest {
    /// Start music with an intro leading into a loop.
    Music {
        /// Intro track.
        intro: String,
        /// Loop track.
        loop_track: String,
    },
    /// Stop music.
    MusicStop,
    /// Remap a shader with a time offset.
    ShaderRemap {
        /// Original shader.
        original: String,
        /// Replacement shader.
        replacement: String,
        /// Time offset.
        time_offset: f64,
    },
}

fn json_escape(text: &str, out: &mut String) {
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ if (ch as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", ch as u32)),
            _ => out.push(ch),
        }
    }
}

/// Checkpoint key for an optional presentation owner (`presentationOwnerKey`).
#[must_use]
pub fn presentation_owner_key(owner: Option<&PresentationOwner>) -> String {
    match owner {
        None => "primary".to_string(),
        Some(owner) => {
            let mut key = String::from("[\"");
            json_escape(
                &format!("{}:{}", owner.provider.namespace, owner.provider.name),
                &mut key,
            );
            key.push_str(&format!("\",{}]", owner.generation));
            key
        }
    }
}

/// Whether an optional owner matches an activation (`samePresentationOwner`).
#[must_use]
pub fn same_presentation_owner(left: Option<&PresentationOwner>, right: &PresentationOwner) -> bool {
    matches!(left, Some(left) if left.provider == right.provider && left.generation == right.generation)
}

// Component client outputs (donor `src/world/session/mod-client-outputs.ts`).
// Original component checkpoints own the fields; leases retain only their
// live publication.

fn provider_name(provider: &ProviderId) -> String {
    format!("{}:{}", provider.namespace, provider.name)
}

fn output_channel_name(channel: ModClientOutputChannel) -> &'static str {
    match channel {
        ModClientOutputChannel::ViewOffset => "view-offset",
        ModClientOutputChannel::MovementMode => "movement-mode",
        ModClientOutputChannel::Stance => "stance",
        ModClientOutputChannel::BodyShape => "body-shape",
    }
}

fn output_kind(output: &ModClientOutput) -> ModClientOutputChannel {
    match output {
        ModClientOutput::ViewOffset(_) => ModClientOutputChannel::ViewOffset,
        ModClientOutput::MovementMode(_) => ModClientOutputChannel::MovementMode,
        ModClientOutput::Stance(_) => ModClientOutputChannel::Stance,
        ModClientOutput::BodyShape(_) => ModClientOutputChannel::BodyShape,
    }
}

fn declaration_kind<Scalar, Vector>(
    declaration: &ModClientOutputDeclaration<Scalar, Vector>,
) -> ModClientOutputChannel {
    match declaration {
        ModClientOutputDeclaration::BodyShape { .. } => ModClientOutputChannel::BodyShape,
        ModClientOutputDeclaration::ViewOffsetField { .. } | ModClientOutputDeclaration::ViewHeight { .. } => {
            ModClientOutputChannel::ViewOffset
        }
        ModClientOutputDeclaration::MovementMode { .. } => ModClientOutputChannel::MovementMode,
        ModClientOutputDeclaration::Stance { .. } => ModClientOutputChannel::Stance,
    }
}

struct OutputEntry {
    owner: ProviderId,
    lease: u64,
    values: HashMap<ActorId, ModClientOutput>,
}

/// Lease publishing one owner's declared channels.
pub struct ModClientOutputLease {
    live: Rc<dyn Fn(&ActorId) -> bool>,
    owners: Rc<RefCell<HashMap<ModClientOutputChannel, OutputEntry>>>,
    owner: ProviderId,
    channels: Vec<ModClientOutputChannel>,
    id: u64,
    active: Cell<bool>,
}

impl std::fmt::Debug for ModClientOutputLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModClientOutputLease")
            .field("owner", &self.owner)
            .field("channels", &self.channels)
            .field("active", &self.active.get())
            .finish()
    }
}

impl ModClientOutputLease {
    fn current(&self) -> Result<(), ContractError> {
        if !self.active.get() {
            return Err(invalid("Component client output owner is retired".to_string()));
        }
        let owners = self.owners.borrow();
        let retired = self
            .channels
            .iter()
            .any(|channel| owners.get(channel).is_none_or(|entry| entry.lease != self.id));
        if retired {
            return Err(invalid("Component client output owner is retired".to_string()));
        }
        Ok(())
    }

    /// Publish detached values after an original invocation commits its source fields.
    pub fn publish(&self, actor: &ActorId, outputs: &[ModClientOutput]) -> Result<(), ContractError> {
        self.current()?;
        if !(self.live)(actor) {
            return Err(invalid(
                "Component client output requires the current client actor".to_string(),
            ));
        }
        let kinds: HashSet<ModClientOutputChannel> = outputs.iter().map(output_kind).collect();
        if outputs.len() != self.channels.len() || kinds.len() != outputs.len() {
            return Err(invalid(
                "Client output publication differs from its declared channels".to_string(),
            ));
        }
        let mut validated = Vec::with_capacity(outputs.len());
        for output in outputs {
            if !self.channels.contains(&output_kind(output)) {
                return Err(invalid("Undeclared component client output".to_string()));
            }
            validated.push(detached_output(*output)?);
        }
        let mut owners = self.owners.borrow_mut();
        for value in validated {
            if let Some(entry) = owners.get_mut(&output_kind(&value)) {
                entry.values.insert(actor.clone(), value);
            }
        }
        Ok(())
    }

    /// Drop one actor's publication.
    pub fn release(&self, actor: &ActorId) {
        let mut owners = self.owners.borrow_mut();
        for channel in &self.channels {
            if let Some(entry) = owners.get_mut(channel) {
                entry.values.remove(actor);
            }
        }
    }

    /// Retire the lease, clearing its channels when still owned.
    pub fn close(&self) {
        if !self.active.get() {
            return;
        }
        self.active.set(false);
        let mut owners = self.owners.borrow_mut();
        for channel in &self.channels {
            let owned = owners.get(channel).is_some_and(|entry| entry.lease == self.id);
            if owned {
                owners.remove(channel);
            }
        }
    }
}

/// Live publication of declared component client outputs.
pub struct ModClientOutputs {
    live: Rc<dyn Fn(&ActorId) -> bool>,
    owners: Rc<RefCell<HashMap<ModClientOutputChannel, OutputEntry>>>,
    next_lease: Cell<u64>,
}

impl ModClientOutputs {
    /// Create an output table; `live` reports the current client actor.
    pub fn new(live: impl Fn(&ActorId) -> bool + 'static) -> Self {
        Self {
            live: Rc::new(live),
            owners: Rc::new(RefCell::new(HashMap::new())),
            next_lease: Cell::new(0),
        }
    }

    /// Claim channels for one owner.
    pub fn claim(
        &self,
        owner: ProviderId,
        channels: &[ModClientOutputChannel],
    ) -> Result<ModClientOutputLease, ContractError> {
        let unique: HashSet<ModClientOutputChannel> = channels.iter().copied().collect();
        if unique.len() != channels.len() {
            return Err(invalid("Duplicate component client output channel".to_string()));
        }
        let mut owners = self.owners.borrow_mut();
        for channel in channels {
            if let Some(previous) = owners.get(channel) {
                return Err(invalid(format!(
                    "Client {} is already owned by {}; {} cannot also claim it",
                    output_channel_name(*channel),
                    provider_name(&previous.owner),
                    provider_name(&owner),
                )));
            }
        }
        let id = self.next_lease.get();
        self.next_lease.set(id + 1);
        for channel in channels {
            owners.insert(
                *channel,
                OutputEntry {
                    owner: owner.clone(),
                    lease: id,
                    values: HashMap::new(),
                },
            );
        }
        Ok(ModClientOutputLease {
            live: self.live.clone(),
            owners: self.owners.clone(),
            owner,
            channels: channels.to_vec(),
            id,
            active: Cell::new(true),
        })
    }

    /// Read one actor's merged movement outputs.
    #[must_use]
    pub fn read(&self, actor: &ActorId) -> Option<ModClientMovementOutputs> {
        let owners = self.owners.borrow();
        if owners.is_empty() {
            return None;
        }
        let view = owners
            .get(&ModClientOutputChannel::ViewOffset)
            .and_then(|entry| entry.values.get(actor));
        let mode = owners
            .get(&ModClientOutputChannel::MovementMode)
            .and_then(|entry| entry.values.get(actor));
        let stance = owners
            .get(&ModClientOutputChannel::Stance)
            .and_then(|entry| entry.values.get(actor));
        let body = owners
            .get(&ModClientOutputChannel::BodyShape)
            .and_then(|entry| entry.values.get(actor));
        if view.is_none() && mode.is_none() && stance.is_none() && body.is_none() || !(self.live)(actor) {
            return None;
        }
        Some(ModClientMovementOutputs {
            view_offset: view.and_then(|output| match output {
                ModClientOutput::ViewOffset(value) => Some(*value),
                _ => None,
            }),
            mode: mode.and_then(|output| match output {
                ModClientOutput::MovementMode(value) => Some(*value),
                _ => None,
            }),
            stance: stance.and_then(|output| match output {
                ModClientOutput::Stance(value) => Some(*value),
                _ => None,
            }),
            body_bounds: body.and_then(|output| match output {
                ModClientOutput::BodyShape(value) => Some(*value),
                _ => None,
            }),
        })
    }

    /// Drop one actor's publication across all owners.
    pub fn release(&self, actor: &ActorId) {
        for entry in self.owners.borrow_mut().values_mut() {
            entry.values.remove(actor);
        }
    }

    /// Clear every publication.
    pub fn close(&self) {
        let mut owners = self.owners.borrow_mut();
        for entry in owners.values_mut() {
            entry.values.clear();
        }
        owners.clear();
    }
}

fn checked_vector(value: Vec3) -> Result<Vec3, ContractError> {
    if ![value.x, value.y, value.z]
        .iter()
        .all(|component| component.is_finite())
    {
        return Err(invalid("Client output vector must be finite".to_string()));
    }
    Ok(value)
}

fn detached_output(output: ModClientOutput) -> Result<ModClientOutput, ContractError> {
    match output {
        ModClientOutput::BodyShape(bounds) => {
            let min = checked_vector(bounds.min)?;
            let max = checked_vector(bounds.max)?;
            if min.x > max.x || min.y > max.y || min.z > max.z {
                return Err(invalid("Client body output has backwards bounds".to_string()));
            }
            Ok(ModClientOutput::BodyShape(Bounds { min, max }))
        }
        ModClientOutput::ViewOffset(value) => Ok(ModClientOutput::ViewOffset(checked_vector(value)?)),
        ModClientOutput::MovementMode(_) | ModClientOutput::Stance(_) => Ok(output),
    }
}

/// JavaScript `ToInt32` for an integral float in `i32`/`u32` range.
fn to_int_32(value: f64) -> i32 {
    (value as i64) as i32
}

/// Read declared outputs from source fields.
pub fn read_mod_client_outputs<Scalar, Vector>(
    declarations: &[ModClientOutputDeclaration<Scalar, Vector>],
    scalar: impl Fn(&Scalar) -> f64,
    vector: impl Fn(&Vector) -> Vec3,
) -> Result<Vec<ModClientOutput>, ContractError> {
    declarations
        .iter()
        .map(|declaration| match declaration {
            ModClientOutputDeclaration::BodyShape { min, max } => Ok(ModClientOutput::BodyShape(Bounds {
                min: vector(min),
                max: vector(max),
            })),
            ModClientOutputDeclaration::ViewOffsetField { field } => Ok(ModClientOutput::ViewOffset(vector(field))),
            ModClientOutputDeclaration::ViewHeight { height } => Ok(ModClientOutput::ViewOffset(Vec3 {
                x: 0.0,
                y: 0.0,
                z: scalar(height) as f32,
            })),
            ModClientOutputDeclaration::MovementMode { field, mask, values } => {
                let raw = scalar(field);
                if !raw.is_finite()
                    || mask.is_some() && (raw.fract() != 0.0 || raw < f64::from(i32::MIN) || raw > 4_294_967_295.0)
                {
                    return Err(invalid(
                        "Client output requires a finite source value, integral when masked".to_string(),
                    ));
                }
                let value = mask.map_or(raw, |mask| f64::from((to_int_32(raw) & to_int_32(mask)) as u32));
                let selected = values
                    .iter()
                    .find(|entry| entry.value == value)
                    .ok_or_else(|| invalid(format!("Undeclared source movement mode {value}")))?;
                Ok(ModClientOutput::MovementMode(selected.mode))
            }
            ModClientOutputDeclaration::Stance { field, mask, values } => {
                let raw = scalar(field);
                if !raw.is_finite()
                    || mask.is_some() && (raw.fract() != 0.0 || raw < f64::from(i32::MIN) || raw > 4_294_967_295.0)
                {
                    return Err(invalid(
                        "Client output requires a finite source value, integral when masked".to_string(),
                    ));
                }
                let value = mask.map_or(raw, |mask| f64::from((to_int_32(raw) & to_int_32(mask)) as u32));
                let selected = values
                    .iter()
                    .find(|entry| entry.value == value)
                    .ok_or_else(|| invalid(format!("Undeclared source client stance {value}")))?;
                Ok(ModClientOutput::Stance(selected.crouched))
            }
        })
        .collect()
}

/// Validate declared outputs against source field checks.
pub fn validate_mod_client_outputs<Scalar, Vector>(
    declarations: &[ModClientOutputDeclaration<Scalar, Vector>],
    scalar: impl Fn(&Scalar) -> Result<(), ContractError>,
    vector: impl Fn(&Vector) -> Result<(), ContractError>,
) -> Result<(), ContractError> {
    let kinds: HashSet<ModClientOutputChannel> = declarations.iter().map(declaration_kind).collect();
    if kinds.len() != declarations.len() {
        return Err(invalid("Duplicate source client output channel".to_string()));
    }
    for declaration in declarations {
        match declaration {
            ModClientOutputDeclaration::BodyShape { min, max } => {
                vector(min)?;
                vector(max)?;
            }
            ModClientOutputDeclaration::ViewOffsetField { field } => {
                vector(field)?;
            }
            ModClientOutputDeclaration::ViewHeight { height } => {
                scalar(height)?;
            }
            ModClientOutputDeclaration::MovementMode { field, mask, values } => {
                scalar(field)?;
                validate_output_values(*mask, &values.iter().map(|entry| entry.value).collect::<Vec<_>>())?;
            }
            ModClientOutputDeclaration::Stance { field, mask, values } => {
                scalar(field)?;
                validate_output_values(*mask, &values.iter().map(|entry| entry.value).collect::<Vec<_>>())?;
            }
        }
    }
    Ok(())
}

fn validate_output_values(mask: Option<f64>, values: &[f64]) -> Result<(), ContractError> {
    if let Some(mask) = mask {
        if mask.fract() != 0.0 || mask <= 0.0 || mask > 4_294_967_295.0 {
            return Err(invalid("Invalid source client output mask".to_string()));
        }
        let masked = to_int_32(mask);
        if values.iter().any(|value| {
            value.fract() != 0.0
                || *value < 0.0
                || *value > 4_294_967_295.0
                || f64::from((to_int_32(*value) & masked) as u32) != *value
        }) {
            return Err(invalid("Source output values escape their declared mask".to_string()));
        }
    }
    let unique: HashSet<u64> = values.iter().map(|value| value.to_bits()).collect();
    if values.is_empty() || values.iter().any(|value| !value.is_finite()) || unique.len() != values.len() {
        return Err(invalid("Ambiguous source client output values".to_string()));
    }
    Ok(())
}

/// Destination claim for source-declared outputs.
pub type OutputClaimFn =
    Box<dyn Fn(ProviderId, Vec<ModClientOutputChannel>) -> Result<ModClientOutputLease, ContractError>>;
/// Source scalar field reader.
pub type OutputScalarFn<Scalar> = Box<dyn Fn(&ActorId, &Scalar) -> f64>;
/// Source vector field reader.
pub type OutputVectorFn<Vector> = Box<dyn Fn(&ActorId, &Vector) -> Vec3>;

/// Source-declared outputs admitted through one lease.
pub struct SourceModClientOutputs<Scalar, Vector> {
    owner: ProviderId,
    declarations: Vec<ModClientOutputDeclaration<Scalar, Vector>>,
    claim: Option<OutputClaimFn>,
    scalar: OutputScalarFn<Scalar>,
    vector: OutputVectorFn<Vector>,
    lease: Option<ModClientOutputLease>,
    actors: HashSet<ActorId>,
}

impl<Scalar, Vector> SourceModClientOutputs<Scalar, Vector> {
    /// Declare source outputs; a destination owner is required when declared.
    pub fn new(
        owner: ProviderId,
        declarations: Vec<ModClientOutputDeclaration<Scalar, Vector>>,
        claim: Option<
            impl Fn(ProviderId, Vec<ModClientOutputChannel>) -> Result<ModClientOutputLease, ContractError> + 'static,
        >,
        scalar: impl Fn(&ActorId, &Scalar) -> f64 + 'static,
        vector: impl Fn(&ActorId, &Vector) -> Vec3 + 'static,
    ) -> Result<Self, ContractError> {
        if !declarations.is_empty() && claim.is_none() {
            return Err(invalid(
                "Declared client outputs require a destination output owner".to_string(),
            ));
        }
        Ok(Self {
            owner,
            declarations,
            claim: claim.map(|claim| {
                Box::new(claim)
                    as Box<
                        dyn Fn(ProviderId, Vec<ModClientOutputChannel>) -> Result<ModClientOutputLease, ContractError>,
                    >
            }),
            scalar: Box::new(scalar),
            vector: Box::new(vector),
            lease: None,
            actors: HashSet::new(),
        })
    }

    /// Whether any outputs are declared.
    #[must_use]
    pub fn enabled(&self) -> bool {
        !self.declarations.is_empty()
    }

    /// Whether an actor has published outputs.
    #[must_use]
    pub fn has(&self, actor: &ActorId) -> bool {
        self.actors.contains(actor)
    }

    /// Publish one actor's outputs, claiming the lease on first use.
    pub fn publish(&mut self, actor: &ActorId) -> Result<(), ContractError> {
        if self.declarations.is_empty() {
            return Ok(());
        }
        let values = read_mod_client_outputs(
            &self.declarations,
            |field| (self.scalar)(actor, field),
            |field| (self.vector)(actor, field),
        )?;
        if self.lease.is_none() {
            let claim = self
                .claim
                .as_ref()
                .ok_or_else(|| invalid("Client output admission is unavailable".to_string()))?;
            let channels = self.declarations.iter().map(declaration_kind).collect::<Vec<_>>();
            self.lease = Some(claim(self.owner.clone(), channels)?);
        }
        let lease = self
            .lease
            .as_ref()
            .ok_or_else(|| invalid("Client output admission is unavailable".to_string()))?;
        lease.publish(actor, &values)?;
        self.actors.insert(actor.clone());
        Ok(())
    }

    /// Release one actor's publication.
    pub fn release(&mut self, actor: &ActorId) {
        if let Some(lease) = self.lease.as_ref() {
            lease.release(actor);
        }
        self.actors.remove(actor);
    }

    /// Release every publication, keeping the lease.
    pub fn clear(&mut self) {
        if let Some(lease) = self.lease.as_ref() {
            for actor in std::mem::take(&mut self.actors) {
                lease.release(&actor);
            }
        } else {
            self.actors.clear();
        }
    }

    /// Retire the lease and forget every actor.
    pub fn close(&mut self) {
        if let Some(lease) = self.lease.as_ref() {
            lease.close();
        }
        self.lease = None;
        self.actors.clear();
    }
}

// Component match state (donor `src/world/session/mod-match.ts`).
// Bindings borrow original source storage; the session retains no second
// score or objective state.

/// Borrowed live-actor surface used by match bindings (donor
/// `SessionActorRegistry`; only the consulted surface is ported).
pub trait SessionActorRegistry {
    /// Whether an actor is live.
    fn is_live(&self, actor: &ActorId) -> bool;
    /// Resolve a live actor to its owned handle.
    fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor>;
}

/// Live source match player borrowed from original storage.
pub trait SourceMatchPlayer {
    /// Owning provider.
    fn owner(&self) -> &ProviderId;
    /// Shared team identity.
    fn team(&self) -> Option<String>;
    /// Score.
    fn score(&self) -> f64;
    /// Assign the shared team.
    fn set_team(&mut self, team: Option<String>);
    /// Assign the score.
    fn set_score(&mut self, score: f64);
}

/// Shared handle to a live source player.
pub type MatchPlayerHandle = Rc<RefCell<dyn SourceMatchPlayer>>;
/// Player resolver for one owner.
pub type MatchPlayerResolver = Rc<dyn Fn(&ActorId) -> Option<MatchPlayerHandle>>;

/// Live source objective state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceObjectiveState {
    /// Objective stage.
    pub stage: String,
    /// Completion.
    pub complete: bool,
    /// Carrier actor.
    pub carrier: Option<ActorId>,
    /// Target actor.
    pub target: Option<ActorId>,
}

/// Objective change request (completion is source-owned).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceObjectiveChange {
    /// Objective stage.
    pub stage: String,
    /// Carrier actor.
    pub carrier: Option<ActorId>,
    /// Target actor.
    pub target: Option<ActorId>,
}

/// Live source objective binding borrowed from its owner.
pub trait SourceObjectiveBinding {
    /// Owning provider.
    fn owner(&self) -> &ProviderId;
    /// Objective identity.
    fn id(&self) -> &ObjectiveId;
    /// Whether the objective gates campaign progress.
    fn campaign_gate(&self) -> bool;
    /// Whether the objective is a bot goal.
    fn bot_goal(&self) -> bool;
    /// Read the live state.
    fn read(&self) -> SourceObjectiveState;
    /// Apply a change request.
    fn change(&self, request: SourceObjectiveChange);
}

/// Objective view with its bot-goal flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceObjectiveView {
    /// Objective identity.
    pub id: ObjectiveId,
    /// Live state.
    pub state: SourceObjectiveState,
    /// Whether the objective is a bot goal.
    pub bot_goal: bool,
}

/// Match services borrowed from admitted sources (donor `SourceMatchServices`).
pub trait SourceMatchServices {
    /// Resolve a live actor's player.
    fn player(&self, actor: &ActorId) -> Result<Option<MatchPlayerHandle>, ContractError>;
    /// Bind one owner's player resolver, returning its unbind.
    fn bind_source(&self, owner: ProviderId, resolve: MatchPlayerResolver) -> Result<Box<dyn Fn()>, ContractError>;
    /// Read one objective's live state.
    fn objective(&self, id: &ObjectiveId) -> Option<SourceObjectiveState>;
    /// Apply an objective change, returning the live state when still owned.
    fn change_objective(
        &self,
        id: &ObjectiveId,
        request: SourceObjectiveChange,
    ) -> Result<Option<SourceObjectiveState>, ContractError>;
    /// Bind one objective owner, returning its unbind.
    fn bind_objective(&self, binding: Rc<dyn SourceObjectiveBinding>) -> Result<Box<dyn Fn()>, ContractError>;
    /// Live views of every bound objective.
    fn objectives(&self) -> Vec<SourceObjectiveView>;
    /// Campaign gates with their satisfaction.
    fn gates(&self) -> Vec<MissionGate>;
}

struct MatchStateInner {
    actors: Box<dyn SessionActorRegistry>,
    primary: MatchPlayerResolver,
    sources: HashMap<ProviderId, (u64, MatchPlayerResolver)>,
    channels: HashMap<ObjectiveId, Rc<dyn SourceObjectiveBinding>>,
    next: u64,
    closed: bool,
}

/// Match bindings over borrowed source storage.
#[derive(Clone)]
pub struct ModMatchState {
    inner: Rc<RefCell<MatchStateInner>>,
}

impl ModMatchState {
    /// Create match services over borrowed actors with a primary resolver.
    pub fn new(actors: Box<dyn SessionActorRegistry>, primary: MatchPlayerResolver) -> Self {
        Self {
            inner: Rc::new(RefCell::new(MatchStateInner {
                actors,
                primary,
                sources: HashMap::new(),
                channels: HashMap::new(),
                next: 0,
                closed: false,
            })),
        }
    }

    fn read_binding(binding: &Rc<dyn SourceObjectiveBinding>, inner: &MatchStateInner) -> Option<SourceObjectiveState> {
        let value = binding.read();
        let current = !inner.closed
            && inner
                .channels
                .get(binding.id())
                .is_some_and(|current| Rc::ptr_eq(current, binding));
        if !current {
            return None;
        }
        Some(SourceObjectiveState {
            stage: value.stage,
            complete: value.complete,
            carrier: value.carrier.filter(|actor| inner.actors.is_live(actor)),
            target: value.target.filter(|actor| inner.actors.is_live(actor)),
        })
    }

    /// Resolve a live actor's player.
    pub fn player(&self, actor: &ActorId) -> Result<Option<MatchPlayerHandle>, ContractError> {
        let (owned, resolver, primary) = {
            let inner = self.inner.borrow();
            if inner.closed || !inner.actors.is_live(actor) {
                return Ok(None);
            }
            let owned = inner.actors.resolve_owned(actor);
            let resolver = owned
                .as_ref()
                .and_then(|owned| inner.sources.get(owned.owner()).map(|(_, resolver)| resolver.clone()));
            (owned, resolver, inner.primary.clone())
        };
        let found = resolver
            .as_ref()
            .and_then(|resolver| resolver(actor))
            .or_else(|| primary(actor));
        if let Some(handle) = &found {
            let owner = handle.borrow().owner().clone();
            let same = match &owned {
                Some(owned) => owner == *owned.owner(),
                None => false,
            };
            if !same {
                return Err(invalid(
                    "Match player belongs to another original actor owner".to_string(),
                ));
            }
        }
        Ok(found)
    }

    /// Bind one owner's player resolver, returning its unbind.
    pub fn bind_source(&self, owner: ProviderId, resolve: MatchPlayerResolver) -> Result<Box<dyn Fn()>, ContractError> {
        let id = {
            let mut inner = self.inner.borrow_mut();
            if inner.closed || inner.sources.contains_key(&owner) {
                return Err(invalid(format!(
                    "Match source {} is already bound or closed",
                    provider_name(&owner)
                )));
            }
            let id = inner.next;
            inner.next += 1;
            inner.sources.insert(owner.clone(), (id, resolve));
            id
        };
        let inner = self.inner.clone();
        Ok(Box::new(move || {
            let mut inner = inner.borrow_mut();
            if inner.sources.get(&owner).is_some_and(|(current, _)| *current == id) {
                inner.sources.remove(&owner);
            }
        }))
    }

    /// Bind one objective owner, returning its unbind.
    pub fn bind_objective(&self, binding: Rc<dyn SourceObjectiveBinding>) -> Result<Box<dyn Fn()>, ContractError> {
        {
            let mut inner = self.inner.borrow_mut();
            if inner.closed {
                return Err(invalid("Match objective owner is closed".to_string()));
            }
            if let Some(previous) = inner.channels.get(binding.id()) {
                return Err(invalid(format!(
                    "Objective {} is owned by {}; {} cannot claim it",
                    binding.id(),
                    provider_name(previous.owner()),
                    provider_name(binding.owner()),
                )));
            }
            inner.channels.insert(binding.id().clone(), binding.clone());
        }
        let inner = self.inner.clone();
        let id = binding.id().clone();
        Ok(Box::new(move || {
            let mut inner = inner.borrow_mut();
            if inner
                .channels
                .get(&id)
                .is_some_and(|current| Rc::ptr_eq(current, &binding))
            {
                inner.channels.remove(&id);
            }
        }))
    }

    /// Read one objective's live state.
    #[must_use]
    pub fn objective(&self, id: &ObjectiveId) -> Option<SourceObjectiveState> {
        let inner = self.inner.borrow();
        let binding = inner.channels.get(id)?.clone();
        Self::read_binding(&binding, &inner)
    }

    /// Apply an objective change, returning the live state when still owned.
    pub fn change_objective(
        &self,
        id: &ObjectiveId,
        request: SourceObjectiveChange,
    ) -> Result<Option<SourceObjectiveState>, ContractError> {
        let binding = {
            let inner = self.inner.borrow();
            let Some(binding) = inner.channels.get(id).cloned() else {
                return Err(invalid(format!("Objective {id} has no admitted source owner")));
            };
            for actor in [&request.carrier, &request.target].into_iter().flatten() {
                if !inner.actors.is_live(actor) {
                    return Err(invalid("Objective change references a retired actor".to_string()));
                }
            }
            binding
        };
        binding.change(request);
        let inner = self.inner.borrow();
        if inner
            .channels
            .get(id)
            .is_some_and(|current| Rc::ptr_eq(current, &binding))
        {
            Ok(Self::read_binding(&binding, &inner))
        } else {
            Ok(None)
        }
    }

    /// Live views of every bound objective.
    #[must_use]
    pub fn objectives(&self) -> Vec<SourceObjectiveView> {
        let inner = self.inner.borrow();
        let bindings: Vec<Rc<dyn SourceObjectiveBinding>> = inner.channels.values().cloned().collect();
        bindings
            .into_iter()
            .filter_map(|binding| {
                Self::read_binding(&binding, &inner).map(|state| SourceObjectiveView {
                    id: binding.id().clone(),
                    state,
                    bot_goal: binding.bot_goal(),
                })
            })
            .collect()
    }

    /// Campaign gates with their satisfaction.
    #[must_use]
    pub fn gates(&self) -> Vec<MissionGate> {
        let inner = self.inner.borrow();
        let bindings: Vec<Rc<dyn SourceObjectiveBinding>> = inner
            .channels
            .values()
            .filter(|binding| binding.campaign_gate())
            .cloned()
            .collect();
        bindings
            .into_iter()
            .filter_map(|binding| {
                Self::read_binding(&binding, &inner).map(|state| MissionGate {
                    objective: binding.id().clone(),
                    satisfied: state.complete,
                })
            })
            .collect()
    }

    /// Release every binding. Idempotent.
    pub fn close(&self) {
        let mut inner = self.inner.borrow_mut();
        if inner.closed {
            return;
        }
        inner.closed = true;
        inner.sources.clear();
        inner.channels.clear();
    }
}

impl SourceMatchServices for ModMatchState {
    fn player(&self, actor: &ActorId) -> Result<Option<MatchPlayerHandle>, ContractError> {
        ModMatchState::player(self, actor)
    }

    fn bind_source(&self, owner: ProviderId, resolve: MatchPlayerResolver) -> Result<Box<dyn Fn()>, ContractError> {
        ModMatchState::bind_source(self, owner, resolve)
    }

    fn objective(&self, id: &ObjectiveId) -> Option<SourceObjectiveState> {
        ModMatchState::objective(self, id)
    }

    fn change_objective(
        &self,
        id: &ObjectiveId,
        request: SourceObjectiveChange,
    ) -> Result<Option<SourceObjectiveState>, ContractError> {
        ModMatchState::change_objective(self, id, request)
    }

    fn bind_objective(&self, binding: Rc<dyn SourceObjectiveBinding>) -> Result<Box<dyn Fn()>, ContractError> {
        ModMatchState::bind_objective(self, binding)
    }

    fn objectives(&self) -> Vec<SourceObjectiveView> {
        ModMatchState::objectives(self)
    }

    fn gates(&self) -> Vec<MissionGate> {
        ModMatchState::gates(self)
    }
}

// Component command routing (donor `src/world/session/mod-commands.ts`).
// One registry belongs to one prepared world; the application supplies its
// staged or published buffer. Producer instances are `u64` tokens (donor
// `symbol`s); script reads are synchronous.

/// Game-module producer tag.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModCommandProducer {
    /// Admitted module identity.
    pub module: ModuleIdentity,
    /// Owning instance token.
    pub instance: u64,
}

/// Command source with an optional game-module producer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModCommandSource {
    /// Owning session.
    pub session: SessionId,
    /// Input origin.
    pub origin: CommandOrigin,
    /// Game-module producer, when the call comes from a bound component.
    pub producer: Option<ModCommandProducer>,
}

/// Cvar registry bound to its owning session (`qa-core` registries are
/// session-agnostic; the donor registry carries its context).
pub struct ModCommandCvars {
    /// Owning session.
    pub session: SessionId,
    /// Registry.
    pub registry: Rc<RefCell<CvarRegistry>>,
}

/// Component command handler, called only for its own entries.
pub type ModCommandInvokeFn = Rc<dyn Fn(&ModCommandInvocation) -> bool>;
/// Component script reader, synchronous.
pub type ModCommandReadScriptFn = Rc<dyn Fn(&str) -> Option<String>>;

/// One component's command binding.
pub struct ModCommandBinding {
    /// Component selection.
    pub selection: ModSelection,
    /// Admitted module identity.
    pub module: ModuleIdentity,
    /// Component cvars.
    pub cvars: ModCommandCvars,
    /// Declared command names, if any.
    pub names: Option<Vec<String>>,
    /// Command handler.
    pub invoke: ModCommandInvokeFn,
    /// Script reader.
    pub read_script: Option<ModCommandReadScriptFn>,
}

/// One command invocation routed to a component.
#[derive(Debug, Clone)]
pub struct ModCommandInvocation {
    /// Argument vector.
    pub argv: Vec<String>,
    /// Raw text after the first token.
    pub args_text: String,
    /// Invocation source.
    pub source: ModCommandSource,
    /// Dispatch dialect.
    pub dialect: Dialect,
    active: Cell<bool>,
}

impl ModCommandInvocation {
    /// Create an active invocation.
    #[must_use]
    pub fn new(argv: Vec<String>, args_text: String, source: ModCommandSource, dialect: Dialect) -> Self {
        Self {
            argv,
            args_text,
            source,
            dialect,
            active: Cell::new(true),
        }
    }

    /// Fail once the invocation frame retires.
    pub fn assert_active(&self) -> Result<(), ContractError> {
        if self.active.get() {
            Ok(())
        } else {
            Err(invalid("Command invocation is no longer active".to_string()))
        }
    }

    /// Retire the invocation frame.
    pub fn retire(&self) {
        self.active.set(false);
    }
}

/// Staged or published command buffer behind one registry. Wiring to the
/// `qa-core` buffer awaits its producer surface; tests and callers supply
/// implementations of this donor surface.
pub trait ModCommandBuffer {
    /// Owning session.
    fn session(&self) -> &SessionId;
    /// Fallback execution source, if any.
    fn execution_source(&self) -> Option<ModCommandSource>;
    /// Queue text behind the pending program.
    fn append(&mut self, text: &str, source: &ModCommandSource, dialect: Dialect) -> Result<(), ContractError>;
    /// Queue text ahead of the pending program.
    fn insert(&mut self, text: &str, source: &ModCommandSource, dialect: Dialect) -> Result<(), ContractError>;
    /// Execute text immediately, returning the executed count.
    fn execute_now(
        &mut self,
        text: Option<&str>,
        source: &ModCommandSource,
        dialect: Dialect,
    ) -> Result<usize, ContractError>;
    /// Discard one producer's queued text.
    fn discard_producer(&mut self, instance: u64);
}

/// Shared command buffer handle.
pub type ModCommandBufferHandle = Rc<RefCell<dyn ModCommandBuffer>>;
/// Staged or published buffer supplier.
pub type ModCommandsBufferFn = Rc<dyn Fn() -> Option<ModCommandBufferHandle>>;

/// Pending queue direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingKind {
    /// Queue behind the pending program.
    Append,
    /// Queue ahead of the pending program.
    Insert,
}

struct PendingCommand {
    kind: PendingKind,
    text: String,
    source: ModCommandSource,
    dialect: Dialect,
}

struct CommandEntry {
    binding: ModCommandBinding,
    port: Rc<RefCell<ModCommandPort>>,
}

struct CommandEntryView {
    cvars: Rc<RefCell<CvarRegistry>>,
    names: Option<Vec<String>>,
    invoke: ModCommandInvokeFn,
    read_script: Option<ModCommandReadScriptFn>,
}

struct CommandsInner {
    context: ModCommandSource,
    commands: ModCommandsBufferFn,
    entries: HashMap<u64, CommandEntry>,
    selections: HashMap<String, u64>,
    pending: Vec<PendingCommand>,
    next_instance: u64,
}

/// Component command registry for one prepared world.
#[derive(Clone)]
pub struct ModCommands {
    inner: Rc<RefCell<CommandsInner>>,
}

impl ModCommands {
    /// Create a registry; `commands` supplies the staged or published buffer.
    pub fn new(context: ModCommandSource, commands: ModCommandsBufferFn) -> Self {
        Self {
            inner: Rc::new(RefCell::new(CommandsInner {
                context,
                commands,
                entries: HashMap::new(),
                selections: HashMap::new(),
                pending: Vec::new(),
                next_instance: 0,
            })),
        }
    }

    fn submit(
        &self,
        kind: PendingKind,
        text: &str,
        source: &ModCommandSource,
        dialect: Dialect,
    ) -> Result<(), ContractError> {
        let commands = {
            let inner = self.inner.borrow();
            if source.session != inner.context.session {
                return Err(invalid("Staged command belongs to another session".to_string()));
            }
            (inner.commands)()
        };
        match commands {
            None => {
                self.inner.borrow_mut().pending.push(PendingCommand {
                    kind,
                    text: text.to_string(),
                    source: source.clone(),
                    dialect,
                });
                Ok(())
            }
            Some(buffer) => {
                self.flush()?;
                match kind {
                    PendingKind::Append => buffer.borrow_mut().append(text, source, dialect),
                    PendingKind::Insert => buffer.borrow_mut().insert(text, source, dialect),
                }
            }
        }
    }

    /// Queue text behind the pending program.
    pub fn append(&self, text: &str, source: &ModCommandSource, dialect: Dialect) -> Result<(), ContractError> {
        self.submit(PendingKind::Append, text, source, dialect)
    }

    /// Queue text ahead of the pending program.
    pub fn insert(&self, text: &str, source: &ModCommandSource, dialect: Dialect) -> Result<(), ContractError> {
        self.submit(PendingKind::Insert, text, source, dialect)
    }

    /// Drain queued text into the prepared buffer.
    pub fn flush(&self) -> Result<(), ContractError> {
        if self.inner.borrow().pending.is_empty() {
            return Ok(());
        }
        let commands = (self.inner.borrow().commands)();
        let Some(buffer) = commands else {
            return Err(invalid(
                "Source commands require the candidate's prepared command buffer".to_string(),
            ));
        };
        let pending = std::mem::take(&mut self.inner.borrow_mut().pending);
        for entry in pending {
            match entry.kind {
                PendingKind::Append => buffer.borrow_mut().append(&entry.text, &entry.source, entry.dialect)?,
                PendingKind::Insert => buffer.borrow_mut().insert(&entry.text, &entry.source, entry.dialect)?,
            }
        }
        Ok(())
    }

    /// Bind one component's commands, owned by `resources`.
    pub fn bind(
        &self,
        binding: ModCommandBinding,
        resources: Rc<RefCell<ResourceScope>>,
    ) -> Result<Rc<RefCell<ModCommandPort>>, ContractError> {
        resources
            .borrow_mut()
            .assert_open()
            .map_err(|error| invalid(error.to_string()))?;
        let key = mod_selection_key(&binding.selection)?;
        if binding.module.id != mod_instance_provider(&binding.selection)? {
            return Err(invalid(
                "Mod commands require their component's module identity".to_string(),
            ));
        }
        let mut inner = self.inner.borrow_mut();
        if binding.cvars.session != inner.context.session {
            return Err(invalid("Mod command cvars belong to another session".to_string()));
        }
        if inner.selections.contains_key(&key) {
            return Err(invalid(format!("Mod commands are already bound: {key}")));
        }
        let instance = inner.next_instance;
        inner.next_instance += 1;
        let port = Rc::new(RefCell::new(ModCommandPort {
            producer: ModCommandProducer {
                module: binding.module.clone(),
                instance,
            },
            key: key.clone(),
            cvars: binding.cvars.registry.clone(),
            resources: resources.clone(),
            resources_name: resources.borrow().name().to_string(),
            commands: inner.commands.clone(),
            registry: Rc::downgrade(&self.inner),
            buffers: RefCell::new(Vec::new()),
            closed: Cell::new(false),
        }));
        inner.entries.insert(
            instance,
            CommandEntry {
                binding,
                port: port.clone(),
            },
        );
        inner.selections.insert(key, instance);
        drop(inner);
        resources
            .borrow_mut()
            .own(port.clone())
            .map_err(|error| invalid(error.to_string()))?;
        Ok(port)
    }

    fn entry(&self, source: &ModCommandSource) -> Result<Option<CommandEntryView>, ContractError> {
        let Some(producer) = source.producer.as_ref() else {
            return Ok(None);
        };
        let inner = self.inner.borrow();
        if source.session != inner.context.session {
            return Err(invalid("Mod command source belongs to another session".to_string()));
        }
        let Some(entry) = inner.entries.get(&producer.instance) else {
            return Err(invalid(
                "Mod command producer is no longer bound to this world".to_string(),
            ));
        };
        let expected = &entry.binding.module;
        if producer.module.id != expected.id
            || producer.module.artifact_path != expected.artifact_path
            || producer.module.digest != expected.digest
            || producer.module.revision != expected.revision
        {
            return Err(invalid(
                "Mod command source differs from its admitted module".to_string(),
            ));
        }
        Ok(Some(CommandEntryView {
            cvars: entry.binding.cvars.registry.clone(),
            names: entry.binding.names.clone(),
            invoke: entry.binding.invoke.clone(),
            read_script: entry.binding.read_script.clone(),
        }))
    }

    /// Resolve the registry backing a source.
    pub fn cvars(&self, source: &ModCommandSource) -> Result<Option<Rc<RefCell<CvarRegistry>>>, ContractError> {
        Ok(self.entry(source)?.map(|view| view.cvars))
    }

    /// Whether a source's producer is still bound.
    pub fn active(&self, source: &ModCommandSource) -> Result<bool, ContractError> {
        let Some(producer) = source.producer.as_ref() else {
            return Ok(false);
        };
        if !self.inner.borrow().entries.contains_key(&producer.instance) {
            return Ok(false);
        }
        Ok(self.entry(source)?.is_some())
    }

    /// Whether a source handles a command name.
    pub fn handles(&self, name: &str, source: &ModCommandSource) -> Result<bool, ContractError> {
        Ok(self.entry(source)?.and_then(|view| view.names).is_some_and(|names| {
            names
                .iter()
                .any(|command| command.to_lowercase() == name.to_lowercase())
        }))
    }

    /// Read a script through a source's binding.
    pub fn read_script(&self, name: &str, source: &ModCommandSource) -> Result<Option<String>, ContractError> {
        let Some(view) = self.entry(source)? else {
            return Ok(None);
        };
        let Some(read) = view.read_script else { return Ok(None) };
        Ok(read(name))
    }

    /// Dispatch an invocation to its source's binding.
    pub fn invoke(&self, command: &ModCommandInvocation) -> Result<bool, ContractError> {
        command.assert_active()?;
        let Some(view) = self.entry(&command.source)? else {
            return Ok(false);
        };
        Ok((view.invoke)(command))
    }

    /// Execute text immediately through one selection's port.
    pub fn execute(
        &self,
        selection: &ModSelection,
        text: &str,
        caller: &ModCommandSource,
    ) -> Result<usize, ContractError> {
        let key = mod_selection_key(selection)?;
        let port = {
            let inner = self.inner.borrow();
            let Some(instance) = inner.selections.get(&key) else {
                return Err(invalid(format!("Mod commands are unavailable: {key}")));
            };
            inner.entries.get(instance).map(|entry| entry.port.clone())
        };
        let Some(port) = port else {
            return Err(invalid(format!("Mod commands are unavailable: {key}")));
        };
        let executed = port.borrow().execute_now(Some(text), Some(caller));
        executed
    }
}

/// One component's command port.
pub struct ModCommandPort {
    producer: ModCommandProducer,
    key: String,
    cvars: Rc<RefCell<CvarRegistry>>,
    resources: Rc<RefCell<ResourceScope>>,
    resources_name: String,
    commands: ModCommandsBufferFn,
    registry: Weak<RefCell<CommandsInner>>,
    buffers: RefCell<Vec<ModCommandBufferHandle>>,
    closed: Cell<bool>,
}

impl ModCommandPort {
    /// Port producer tag.
    #[must_use]
    pub fn producer(&self) -> &ModCommandProducer {
        &self.producer
    }

    fn registry_handle(&self) -> Result<ModCommands, ContractError> {
        self.registry
            .upgrade()
            .map(|inner| ModCommands { inner })
            .ok_or_else(|| invalid("Mod command producer is no longer bound to this world".to_string()))
    }

    fn current(&self) -> Result<Option<ModCommandBufferHandle>, ContractError> {
        {
            let resources = self
                .resources
                .try_borrow()
                .map_err(|_| invalid(format!("{} is closed", self.resources_name)))?;
            resources.assert_open().map_err(|error| invalid(error.to_string()))?;
        }
        if self.closed.get() {
            return Err(invalid(format!("Mod commands are closed: {}", self.key)));
        }
        let session = {
            let registry = self.registry_handle()?;
            let session = registry.inner.borrow().context.session.clone();
            session
        };
        let Some(buffer) = (self.commands)() else {
            return Ok(None);
        };
        if buffer.borrow().session() != &session {
            return Err(invalid("Mod command buffer belongs to another session".to_string()));
        }
        let mut tracked = self.buffers.borrow_mut();
        if !tracked.iter().any(|known| Rc::ptr_eq(known, &buffer)) {
            tracked.push(buffer.clone());
        }
        Ok(Some(buffer))
    }

    fn source(
        &self,
        buffer: Option<&ModCommandBufferHandle>,
        caller: Option<&ModCommandSource>,
    ) -> Result<ModCommandSource, ContractError> {
        let registry = self.registry_handle()?;
        let context = registry.inner.borrow().context.clone();
        let base = caller
            .cloned()
            .or_else(|| buffer.and_then(|buffer| buffer.borrow().execution_source()));
        let base = base.unwrap_or(context.clone());
        if base.session != context.session {
            return Err(invalid("Mod command caller belongs to another session".to_string()));
        }
        Ok(ModCommandSource {
            session: base.session,
            origin: base.origin,
            producer: Some(self.producer.clone()),
        })
    }

    fn dialect(&self) -> Dialect {
        self.cvars.borrow().dialect()
    }

    /// Queue text behind the pending program.
    pub fn append(&self, text: &str, caller: Option<&ModCommandSource>) -> Result<(), ContractError> {
        let buffer = self.current()?;
        let source = self.source(buffer.as_ref(), caller)?;
        let dialect = self.dialect();
        self.registry_handle()?.append(text, &source, dialect)
    }

    /// Queue text ahead of the pending program.
    pub fn insert(&self, text: &str, caller: Option<&ModCommandSource>) -> Result<(), ContractError> {
        let buffer = self.current()?;
        let source = self.source(buffer.as_ref(), caller)?;
        let dialect = self.dialect();
        self.registry_handle()?.insert(text, &source, dialect)
    }

    /// Execute text immediately.
    pub fn execute_now(&self, text: Option<&str>, caller: Option<&ModCommandSource>) -> Result<usize, ContractError> {
        let buffer = self.current()?;
        let Some(buffer) = buffer else {
            return Err(invalid(
                "Immediate mod commands require the candidate's prepared command buffer".to_string(),
            ));
        };
        let source = self.source(Some(&buffer), caller)?;
        let dialect = self.dialect();
        let registry = self.registry_handle()?;
        registry.flush()?;
        let executed = buffer.borrow_mut().execute_now(text, &source, dialect);
        executed
    }

    /// Unbind the port, discarding its queued text. Idempotent.
    pub fn close(&self) {
        if self.closed.get() {
            return;
        }
        self.closed.set(true);
        let Some(inner) = self.registry.upgrade() else { return };
        {
            let mut state = inner.borrow_mut();
            state.entries.remove(&self.producer.instance);
            state.selections.remove(&self.key);
            state.pending.retain(|entry| {
                entry
                    .source
                    .producer
                    .as_ref()
                    .is_none_or(|producer| producer.instance != self.producer.instance)
            });
        }
        let mut buffers = self.buffers.borrow_mut();
        if let Some(current) = (self.commands)() {
            if !buffers.iter().any(|known| Rc::ptr_eq(known, &current)) {
                buffers.push(current);
            }
        }
        for buffer in buffers.iter() {
            buffer.borrow_mut().discard_producer(self.producer.instance);
        }
    }
}

impl SessionResource for ModCommandPort {
    fn close(&mut self) -> Result<(), WorldError> {
        ModCommandPort::close(self);
        Ok(())
    }
}

/// Read a `modcmd` console line into its selection and command text.
pub fn read_mod_command(raw: &str) -> Result<(ModSelection, String), ContractError> {
    let tokens = tokenize_command(raw, Dialect::Q3, TextMode::Source).map_err(|error| invalid(error.to_string()))?;
    let text = command_text_tail(raw, Dialect::Q3, 2).map_err(|error| invalid(error.to_string()))?;
    match (tokens.argv.get(1), text.is_empty()) {
        (Some(selected), false) => Ok((read_mod_selection(selected)?, text)),
        _ => Err(invalid("Usage: modcmd PRODUCT/COMPONENT_ID <command>".to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content_id() -> ContentId {
        create_content_id(&ContentIdentity {
            family: GameFamily::Q1,
            edition: "classic".to_string(),
            package: "id1".to_string(),
            revision: "v1".to_string(),
        })
        .unwrap()
    }

    #[test]
    fn builds_validated_ids() {
        assert_eq!(content_id().as_str(), "q1:classic:id1:v1");
        assert!(is_content_id("q2:rerelease:base:2023"));
        assert!(!is_content_id("q4:classic:id1:v1"));
        assert!(!is_content_id("q1:classic:id 1:v1"));
        assert_eq!(create_recipe_id("maps", "v2").unwrap().as_str(), "recipe:maps:v2");
        assert!(create_mount_id("bad name", "x").is_err());
        assert_eq!(
            create_mount_plan_id("plans", "r1").unwrap().as_str(),
            "mount-plan:plans:r1"
        );
    }

    #[test]
    fn builds_digests() {
        let digest = create_content_digest(&"ab".repeat(32)).unwrap();
        assert_eq!(digest.as_str(), &format!("sha256:{}", "ab".repeat(32)));
        assert!(is_content_digest(&format!("sha256:{}", "ab".repeat(32))));
        assert!(!is_content_digest("sha256:xyz"));
        assert!(create_content_digest("xyz").is_err());
    }

    #[test]
    fn encodes_uri_components() {
        assert_eq!(encode_uri_component("a/b c"), "a%2Fb%20c");
        assert_eq!(encode_uri_component("a-_.!~*'()"), "a-_.!~*'()");
    }

    #[test]
    fn builds_resource_ids() {
        let mount = ArchiveMount {
            identity: create_mount_identity(create_mount_id("ns", "pak0").unwrap(), content_id(), 0).unwrap(),
            format: ArchiveFormat::Pak,
            archive_path: "id1/pak0.pak".to_string(),
            archive_digest: create_content_digest(&"cd".repeat(32)).unwrap(),
        };
        let resource = UnresolvedResourceReference {
            requested_path: "maps/e1m1.bsp".to_string(),
            provenance: ResourceProvenance::Archive {
                mount,
                member_path: "maps/e1m1.bsp".to_string(),
                member_index: 3,
            },
            digest: create_content_digest(&"ef".repeat(32)).unwrap(),
            byte_length: 100,
            resolution: ResourceResolution::DefaultOrder {
                plan: create_mount_plan_id("plans", "r1").unwrap(),
                rank: 0,
            },
        };
        let id = create_resource_id(&resource).unwrap();
        assert!(id.as_str().starts_with("resource:q1:classic:id1:v1:"));
        assert!(id.as_str().contains("maps%2Fe1m1.bsp"));
    }

    #[test]
    fn reads_mod_selections() {
        let selection = ModSelection {
            product: "game".to_string(),
            id: "coop/map".to_string(),
        };
        assert_eq!(mod_selection_key(&selection).unwrap(), "game/coop/map");
        let read = read_mod_selection("game/coop/map").unwrap();
        assert_eq!(read, selection);
        assert!(read_mod_selection("noslash").is_err());
        let provider = mod_instance_provider(&selection).unwrap();
        assert_eq!(provider.namespace, "mod");
    }

    #[test]
    fn resolves_equipment_items() {
        let provider = ProviderId::new("q1", "gameplay");
        let contexts = vec![SourceEquipmentContext {
            provider: provider.clone(),
            item: Some("q1:nails".to_string()),
        }];
        assert_eq!(
            source_equipment_item(&contexts, &provider).unwrap(),
            Some("q1:nails".to_string())
        );
        assert!(source_equipment_item(&[], &provider).is_err());
    }

    #[test]
    fn names_source_items() {
        let provider = ProviderId::new("q1", "gameplay");
        let item = SourceItemDefinition {
            item: "q1:shells".to_string(),
            label: "Shells".to_string(),
            source: ProviderReference {
                provider,
                content: content_id(),
            },
            icon: None,
            actions: Vec::new(),
            kind: SourceItemKind::Counter,
        };
        let items = [item];
        let found = source_item_named(&items, "q1:shells", None).unwrap();
        assert!(matches!(found, SourceItemMatch::Match { exact: true, .. }));
        let found = source_item_named(&items, "Shells", None).unwrap();
        assert!(matches!(found, SourceItemMatch::Match { exact: false, .. }));
        assert!(source_item_named(&items, "Nails", None).is_none());
    }

    #[test]
    fn keys_and_matches_presentation_owners() {
        assert_eq!(presentation_owner_key(None), "primary");
        let owner = PresentationOwner {
            provider: ProviderId::new("q3", "game"),
            generation: 2,
        };
        assert_eq!(presentation_owner_key(Some(&owner)), "[\"q3:game\",2]");
        assert!(same_presentation_owner(Some(&owner), &owner));
        assert!(!same_presentation_owner(None, &owner));
        let other = PresentationOwner {
            provider: ProviderId::new("q3", "game"),
            generation: 3,
        };
        assert!(!same_presentation_owner(Some(&other), &owner));
        let foreign = PresentationOwner {
            provider: ProviderId::new("q1", "game"),
            generation: 2,
        };
        assert!(!same_presentation_owner(Some(&foreign), &owner));
        assert!(matches!(
            ComponentPresentationMediaRequest::MusicStop,
            ComponentPresentationMediaRequest::MusicStop
        ));
    }

    #[test]
    fn claims_publishes_and_reads_client_outputs() {
        use qa_core::identity::IdentityOwner;

        let ids = IdentityOwner::create("outputs").unwrap();
        let actor = ids.actor(0, 0);
        let guest = ids.actor(1, 0);
        let live = {
            let actor = actor.clone();
            move |candidate: &ActorId| candidate == &actor
        };
        let table = ModClientOutputs::new(live);
        assert!(table.read(&actor).is_none());
        let owner = ProviderId::new("q3", "game");
        let lease = table
            .claim(
                owner.clone(),
                &[ModClientOutputChannel::ViewOffset, ModClientOutputChannel::Stance],
            )
            .unwrap();
        assert_eq!(
            table
                .claim(owner.clone(), &[ModClientOutputChannel::ViewOffset])
                .unwrap_err()
                .to_string(),
            "Client view-offset is already owned by q3:game; q3:game cannot also claim it"
        );
        assert_eq!(
            table
                .claim(
                    owner.clone(),
                    &[ModClientOutputChannel::Stance, ModClientOutputChannel::Stance]
                )
                .unwrap_err()
                .to_string(),
            "Duplicate component client output channel"
        );
        let view = Vec3 {
            x: 0.0,
            y: 0.0,
            z: 22.0,
        };
        lease
            .publish(
                &actor,
                &[ModClientOutput::ViewOffset(view), ModClientOutput::Stance(true)],
            )
            .unwrap();
        let read = table.read(&actor).unwrap();
        assert_eq!(read.view_offset, Some(view));
        assert_eq!(read.stance, Some(true));
        assert_eq!(read.mode, None);
        assert!(table.read(&guest).is_none());
        assert_eq!(
            lease
                .publish(
                    &guest,
                    &[ModClientOutput::ViewOffset(view), ModClientOutput::Stance(false)]
                )
                .unwrap_err()
                .to_string(),
            "Component client output requires the current client actor"
        );
        assert_eq!(
            lease
                .publish(&actor, &[ModClientOutput::Stance(false)])
                .unwrap_err()
                .to_string(),
            "Client output publication differs from its declared channels"
        );
        assert_eq!(
            lease
                .publish(
                    &actor,
                    &[
                        ModClientOutput::ViewOffset(view),
                        ModClientOutput::MovementMode(ModClientMovementMode::Freeze)
                    ]
                )
                .unwrap_err()
                .to_string(),
            "Undeclared component client output"
        );
        lease.release(&actor);
        assert!(table.read(&actor).is_none());
        lease
            .publish(
                &actor,
                &[ModClientOutput::ViewOffset(view), ModClientOutput::Stance(false)],
            )
            .unwrap();
        lease.close();
        assert_eq!(
            lease
                .publish(
                    &actor,
                    &[ModClientOutput::ViewOffset(view), ModClientOutput::Stance(false)]
                )
                .unwrap_err()
                .to_string(),
            "Component client output owner is retired"
        );
        let next = table.claim(owner, &[ModClientOutputChannel::ViewOffset]).unwrap();
        next.publish(&actor, &[ModClientOutput::ViewOffset(view)]).unwrap();
        assert!(table.read(&actor).is_some());
        table.release(&actor);
        assert!(table.read(&actor).is_none());
        next.close();
    }

    #[test]
    fn rejects_bad_client_vectors_and_bounds() {
        use qa_core::identity::IdentityOwner;

        let ids = IdentityOwner::create("vectors").unwrap();
        let actor = ids.actor(0, 0);
        let table = ModClientOutputs::new(move |_| true);
        let owner = ProviderId::new("q1", "game");
        let lease = table.claim(owner, &[ModClientOutputChannel::BodyShape]).unwrap();
        let bad = Bounds {
            min: Vec3 {
                x: f32::NAN,
                y: 0.0,
                z: 0.0,
            },
            max: Vec3 { x: 1.0, y: 1.0, z: 1.0 },
        };
        assert_eq!(
            lease
                .publish(&actor, &[ModClientOutput::BodyShape(bad)])
                .unwrap_err()
                .to_string(),
            "Client output vector must be finite"
        );
        let backwards = Bounds {
            min: Vec3 { x: 2.0, y: 0.0, z: 0.0 },
            max: Vec3 { x: 1.0, y: 1.0, z: 1.0 },
        };
        assert_eq!(
            lease
                .publish(&actor, &[ModClientOutput::BodyShape(backwards)])
                .unwrap_err()
                .to_string(),
            "Client body output has backwards bounds"
        );
        lease.close();
    }

    #[test]
    fn reads_and_validates_declared_client_outputs() {
        let declarations = vec![
            ModClientOutputDeclaration::ViewHeight::<String, String> {
                height: "viewheight".to_string(),
            },
            ModClientOutputDeclaration::MovementMode {
                field: "move".to_string(),
                mask: Some(3.0),
                values: vec![
                    ModClientMovementModeValue {
                        value: 0.0,
                        mode: ModClientMovementMode::Normal,
                    },
                    ModClientMovementModeValue {
                        value: 3.0,
                        mode: ModClientMovementMode::Noclip,
                    },
                ],
            },
        ];
        validate_mod_client_outputs(&declarations, |_| Ok(()), |_| Ok(())).unwrap();
        let outputs = read_mod_client_outputs(
            &declarations,
            |field| if field == "viewheight" { 22.0 } else { 7.0 },
            |_| Vec3 { x: 0.0, y: 0.0, z: 0.0 },
        )
        .unwrap();
        assert_eq!(outputs.len(), 2);
        assert!(matches!(outputs[0], ModClientOutput::ViewOffset(offset) if offset.z == 22.0));
        assert_eq!(outputs[1], ModClientOutput::MovementMode(ModClientMovementMode::Noclip));
        let bad = vec![ModClientOutputDeclaration::Stance::<String, String> {
            field: "stance".to_string(),
            mask: Some(0.0),
            values: vec![ModClientStanceValue {
                value: 0.0,
                crouched: false,
            }],
        }];
        assert_eq!(
            validate_mod_client_outputs(&bad, |_| Ok(()), |_| Ok(()))
                .unwrap_err()
                .to_string(),
            "Invalid source client output mask"
        );
        let dup = vec![
            ModClientOutputDeclaration::ViewHeight::<String, String> {
                height: "a".to_string(),
            },
            ModClientOutputDeclaration::ViewOffsetField::<String, String> { field: "b".to_string() },
        ];
        assert_eq!(
            validate_mod_client_outputs(&dup, |_| Ok(()), |_| Ok(()))
                .unwrap_err()
                .to_string(),
            "Duplicate source client output channel"
        );
    }

    #[test]
    fn source_outputs_publish_through_one_lease() {
        use qa_core::identity::IdentityOwner;
        use std::rc::Rc;

        let ids = IdentityOwner::create("source-outputs").unwrap();
        let actor = ids.actor(0, 0);
        let table = Rc::new(ModClientOutputs::new(move |_| true));
        let owner = ProviderId::new("q2", "game");
        let claimed = Rc::new(Cell::new(0));
        let seen = claimed.clone();
        let tables = table.clone();
        let mut source = SourceModClientOutputs::new(
            owner.clone(),
            vec![ModClientOutputDeclaration::Stance::<String, String> {
                field: "crouched".to_string(),
                mask: None,
                values: vec![
                    ModClientStanceValue {
                        value: 0.0,
                        crouched: false,
                    },
                    ModClientStanceValue {
                        value: 1.0,
                        crouched: true,
                    },
                ],
            }],
            Some(move |owner: ProviderId, channels: Vec<ModClientOutputChannel>| {
                seen.set(seen.get() + 1);
                tables.claim(owner, &channels)
            }),
            |_, _| 1.0,
            |_, _| Vec3 { x: 0.0, y: 0.0, z: 0.0 },
        )
        .unwrap();
        assert!(source.enabled());
        assert!(!source.has(&actor));
        source.publish(&actor).unwrap();
        assert!(source.has(&actor));
        assert_eq!(claimed.get(), 1);
        source.publish(&actor).unwrap();
        assert_eq!(claimed.get(), 1);
        assert_eq!(table.read(&actor).unwrap().stance, Some(true));
        source.release(&actor);
        assert!(!source.has(&actor));
        assert!(table.read(&actor).is_none());
        source.publish(&actor).unwrap();
        source.clear();
        assert!(!source.has(&actor));
        source.close();
        let idle = SourceModClientOutputs::new(
            owner,
            Vec::<ModClientOutputDeclaration<String, String>>::new(),
            None::<fn(ProviderId, Vec<ModClientOutputChannel>) -> Result<ModClientOutputLease, ContractError>>,
            |_: &ActorId, _: &String| 0.0,
            |_: &ActorId, _: &String| Vec3 { x: 0.0, y: 0.0, z: 0.0 },
        )
        .unwrap();
        assert!(!idle.enabled());
    }

    struct FakeActors {
        live: HashSet<ActorId>,
        owners: HashMap<ActorId, OwnedActor>,
    }

    impl SessionActorRegistry for FakeActors {
        fn is_live(&self, actor: &ActorId) -> bool {
            self.live.contains(actor)
        }

        fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor> {
            self.owners.get(actor).cloned()
        }
    }

    struct FakePlayer {
        owner: ProviderId,
        team: Option<String>,
        score: f64,
    }

    impl SourceMatchPlayer for FakePlayer {
        fn owner(&self) -> &ProviderId {
            &self.owner
        }

        fn team(&self) -> Option<String> {
            self.team.clone()
        }

        fn score(&self) -> f64 {
            self.score
        }

        fn set_team(&mut self, team: Option<String>) {
            self.team = team;
        }

        fn set_score(&mut self, score: f64) {
            self.score = score;
        }
    }

    struct FakeObjective {
        owner: ProviderId,
        id: ObjectiveId,
        campaign_gate: bool,
        bot_goal: bool,
        state: RefCell<SourceObjectiveState>,
    }

    impl SourceObjectiveBinding for FakeObjective {
        fn owner(&self) -> &ProviderId {
            &self.owner
        }

        fn id(&self) -> &ObjectiveId {
            &self.id
        }

        fn campaign_gate(&self) -> bool {
            self.campaign_gate
        }

        fn bot_goal(&self) -> bool {
            self.bot_goal
        }

        fn read(&self) -> SourceObjectiveState {
            self.state.borrow().clone()
        }

        fn change(&self, request: SourceObjectiveChange) {
            let mut state = self.state.borrow_mut();
            state.stage = request.stage;
            state.carrier = request.carrier;
            state.target = request.target;
        }
    }

    #[test]
    fn match_players_prefer_bound_sources() {
        use qa_core::identity::IdentityOwner;

        let ids = IdentityOwner::create("match").unwrap();
        let actor = ids.actor(0, 0);
        let retired = ids.actor(1, 0);
        let owner = ProviderId::new("q3", "match");
        let mut owners = HashMap::new();
        owners.insert(actor.clone(), ids.owned_actor(&actor, owner.clone()).unwrap());
        let actors = FakeActors {
            live: HashSet::from([actor.clone()]),
            owners,
        };
        let primary_player: MatchPlayerHandle = Rc::new(RefCell::new(FakePlayer {
            owner: owner.clone(),
            team: Some("red".to_string()),
            score: 3.0,
        }));
        let primary = primary_player.clone();
        let state = ModMatchState::new(Box::new(actors), Rc::new(move |_| Some(primary.clone())));
        assert!(state.player(&retired).unwrap().is_none());
        assert_eq!(state.player(&actor).unwrap().unwrap().borrow().score(), 3.0);
        let bound_player: MatchPlayerHandle = Rc::new(RefCell::new(FakePlayer {
            owner: owner.clone(),
            team: None,
            score: 9.0,
        }));
        let bound = bound_player.clone();
        let unbind = state
            .bind_source(owner.clone(), Rc::new(move |_| Some(bound.clone())))
            .unwrap();
        assert_eq!(state.player(&actor).unwrap().unwrap().borrow().score(), 9.0);
        assert_eq!(
            state
                .bind_source(owner.clone(), Rc::new(|_| None))
                .err()
                .expect("duplicate source")
                .to_string(),
            "Match source q3:match is already bound or closed"
        );
        unbind();
        unbind();
        assert_eq!(state.player(&actor).unwrap().unwrap().borrow().score(), 3.0);
        let foreign: MatchPlayerHandle = Rc::new(RefCell::new(FakePlayer {
            owner: ProviderId::new("q1", "game"),
            team: None,
            score: 0.0,
        }));
        let unbind = state
            .bind_source(owner, Rc::new(move |_| Some(foreign.clone())))
            .unwrap();
        assert_eq!(
            state.player(&actor).err().expect("owner mismatch").to_string(),
            "Match player belongs to another original actor owner"
        );
        unbind();
        state.close();
        assert!(state.player(&actor).unwrap().is_none());
        state.close();
    }

    #[test]
    fn match_objectives_track_live_bindings() {
        use qa_core::identity::IdentityOwner;

        let ids = IdentityOwner::create("objectives").unwrap();
        let carrier = ids.actor(0, 0);
        let gone = ids.actor(1, 0);
        let owner = ProviderId::new("q1", "campaign");
        let actors = FakeActors {
            live: HashSet::from([carrier.clone()]),
            owners: HashMap::new(),
        };
        let state = ModMatchState::new(Box::new(actors), Rc::new(|_| None));
        let binding: Rc<dyn SourceObjectiveBinding> = Rc::new(FakeObjective {
            owner: owner.clone(),
            id: "q1:key".to_string(),
            campaign_gate: true,
            bot_goal: true,
            state: RefCell::new(SourceObjectiveState {
                stage: "taken".to_string(),
                complete: true,
                carrier: Some(carrier.clone()),
                target: Some(gone.clone()),
            }),
        });
        let unbind = state.bind_objective(binding).unwrap();
        assert_eq!(
            state
                .bind_objective(Rc::new(FakeObjective {
                    owner: ProviderId::new("q3", "match"),
                    id: "q1:key".to_string(),
                    campaign_gate: false,
                    bot_goal: false,
                    state: RefCell::new(SourceObjectiveState {
                        stage: "open".to_string(),
                        complete: false,
                        carrier: None,
                        target: None,
                    }),
                }))
                .err()
                .expect("duplicate objective")
                .to_string(),
            "Objective q1:key is owned by q1:campaign; q3:match cannot claim it"
        );
        let read = state.objective(&"q1:key".to_string()).unwrap();
        assert_eq!(read.carrier, Some(carrier.clone()));
        assert_eq!(read.target, None);
        assert_eq!(
            state
                .change_objective(
                    &"q1:key".to_string(),
                    SourceObjectiveChange {
                        stage: "dropped".to_string(),
                        carrier: Some(gone.clone()),
                        target: None,
                    }
                )
                .expect_err("retired carrier")
                .to_string(),
            "Objective change references a retired actor"
        );
        let changed = state
            .change_objective(
                &"q1:key".to_string(),
                SourceObjectiveChange {
                    stage: "dropped".to_string(),
                    carrier: None,
                    target: Some(carrier.clone()),
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(changed.stage, "dropped");
        assert_eq!(changed.target, Some(carrier));
        assert_eq!(
            state
                .change_objective(
                    &"q1:missing".to_string(),
                    SourceObjectiveChange {
                        stage: "x".to_string(),
                        carrier: None,
                        target: None
                    }
                )
                .expect_err("missing objective")
                .to_string(),
            "Objective q1:missing has no admitted source owner"
        );
        let views = state.objectives();
        assert_eq!(views.len(), 1);
        assert!(views[0].bot_goal);
        let gates = state.gates();
        assert_eq!(gates.len(), 1);
        assert_eq!(gates[0].objective, "q1:key");
        assert!(gates[0].satisfied);
        unbind();
        assert!(state.objective(&"q1:key".to_string()).is_none());
        assert!(state.objectives().is_empty());
        state.close();
    }

    #[derive(Default)]
    struct BufferLogs {
        appended: Vec<(String, ModCommandSource, Dialect)>,
        inserted: Vec<(String, ModCommandSource, Dialect)>,
        executed: Vec<(Option<String>, ModCommandSource, Dialect)>,
        discarded: Vec<u64>,
    }

    struct FakeBuffer {
        session: SessionId,
        logs: Rc<RefCell<BufferLogs>>,
    }

    impl ModCommandBuffer for FakeBuffer {
        fn session(&self) -> &SessionId {
            &self.session
        }

        fn execution_source(&self) -> Option<ModCommandSource> {
            None
        }

        fn append(&mut self, text: &str, source: &ModCommandSource, dialect: Dialect) -> Result<(), ContractError> {
            self.logs
                .borrow_mut()
                .appended
                .push((text.to_string(), source.clone(), dialect));
            Ok(())
        }

        fn insert(&mut self, text: &str, source: &ModCommandSource, dialect: Dialect) -> Result<(), ContractError> {
            self.logs
                .borrow_mut()
                .inserted
                .push((text.to_string(), source.clone(), dialect));
            Ok(())
        }

        fn execute_now(
            &mut self,
            text: Option<&str>,
            source: &ModCommandSource,
            dialect: Dialect,
        ) -> Result<usize, ContractError> {
            self.logs
                .borrow_mut()
                .executed
                .push((text.map(str::to_string), source.clone(), dialect));
            Ok(1)
        }

        fn discard_producer(&mut self, instance: u64) {
            self.logs.borrow_mut().discarded.push(instance);
        }
    }

    fn command_fixture() -> (ModCommands, Rc<RefCell<BufferLogs>>, ModCommandSource) {
        use qa_core::identity::IdentityOwner;

        let ids = IdentityOwner::create("mod-commands").unwrap();
        let context = ModCommandSource {
            session: ids.session().clone(),
            origin: CommandOrigin::ServerConsole,
            producer: None,
        };
        let logs = Rc::new(RefCell::new(BufferLogs::default()));
        let buffer: ModCommandBufferHandle = Rc::new(RefCell::new(FakeBuffer {
            session: context.session.clone(),
            logs: logs.clone(),
        }));
        let registry = ModCommands::new(context.clone(), Rc::new(move || Some(buffer.clone())));
        (registry, logs, context)
    }

    fn command_binding(
        id: &str,
        dialect: Dialect,
        context: &ModCommandSource,
        seen: Rc<RefCell<Vec<String>>>,
    ) -> ModCommandBinding {
        let selection = ModSelection {
            product: "source".to_string(),
            id: id.to_string(),
        };
        let registry = Rc::new(RefCell::new(CvarRegistry::new(dialect)));
        registry.borrow_mut().register("value", id, 0).unwrap();
        ModCommandBinding {
            selection: selection.clone(),
            module: ModuleIdentity {
                id: mod_instance_provider(&selection).unwrap(),
                artifact_path: "game.dll".to_string(),
                digest: create_content_digest(&"a".repeat(64)).unwrap(),
                revision: "original".to_string(),
            },
            cvars: ModCommandCvars {
                session: context.session.clone(),
                registry,
            },
            names: Some(vec!["original".to_string()]),
            invoke: Rc::new(move |command: &ModCommandInvocation| {
                seen.borrow_mut().push(command.argv[0].clone());
                command.argv[0] == "original"
            }),
            read_script: Some(Rc::new(|name: &str| {
                if name == "autoexec.cfg" {
                    Some("echo hi".to_string())
                } else {
                    None
                }
            })),
        }
    }

    #[test]
    fn mod_ports_route_through_producer_sources() {
        let (registry, logs, context) = command_fixture();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let resources = Rc::new(RefCell::new(ResourceScope::new("first")));
        let port = registry
            .bind(
                command_binding("a", Dialect::Q3, &context, seen.clone()),
                resources.clone(),
            )
            .unwrap();
        port.borrow().append("wait\n", None).unwrap();
        port.borrow().insert("echo early\n", None).unwrap();
        assert_eq!(port.borrow().execute_now(Some("version"), None).unwrap(), 1);
        {
            let logs = logs.borrow();
            assert_eq!(logs.appended.len(), 1);
            assert_eq!(logs.appended[0].0, "wait\n");
            assert_eq!(logs.appended[0].2, Dialect::Q3);
            assert_eq!(logs.inserted.len(), 1);
            assert_eq!(logs.executed.len(), 1);
            let producer = logs.appended[0].1.producer.as_ref().unwrap();
            assert_eq!(producer.instance, port.borrow().producer().instance);
            assert_eq!(logs.appended[0].1.session, context.session);
        }
        let source = ModCommandSource {
            session: context.session.clone(),
            origin: CommandOrigin::ServerConsole,
            producer: Some(port.borrow().producer().clone()),
        };
        assert!(registry.active(&source).unwrap());
        assert!(registry.handles("ORIGINAL", &source).unwrap());
        assert!(!registry.handles("other", &source).unwrap());
        let cvars = registry.cvars(&source).unwrap().unwrap();
        assert_eq!(cvars.borrow().variable_string("value"), "a");
        assert_eq!(
            registry.read_script("autoexec.cfg", &source).unwrap(),
            Some("echo hi".to_string())
        );
        let command =
            ModCommandInvocation::new(vec!["original".to_string()], String::new(), source.clone(), Dialect::Q3);
        assert!(registry.invoke(&command).unwrap());
        assert_eq!(*seen.borrow(), vec!["original".to_string()]);
        let retired =
            ModCommandInvocation::new(vec!["original".to_string()], String::new(), source.clone(), Dialect::Q3);
        retired.retire();
        assert_eq!(
            registry.invoke(&retired).expect_err("retired invocation").to_string(),
            "Command invocation is no longer active"
        );
        assert_eq!(
            registry
                .execute(
                    &ModSelection {
                        product: "source".to_string(),
                        id: "a".to_string()
                    },
                    "status",
                    &context
                )
                .unwrap(),
            1
        );
        port.borrow().close();
        assert!(!registry.active(&source).unwrap());
        assert_eq!(
            port.borrow()
                .append("stale\n", None)
                .expect_err("closed port")
                .to_string(),
            "Mod commands are closed: source/a"
        );
        assert_eq!(logs.borrow().discarded, vec![port.borrow().producer().instance]);
        resources.borrow_mut().close().unwrap();
    }

    #[test]
    fn mod_commands_reject_foreign_bindings_and_queue_staged() {
        use qa_core::identity::IdentityOwner;

        let (registry, logs, context) = command_fixture();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let resources = Rc::new(RefCell::new(ResourceScope::new("scope")));
        registry
            .bind(
                command_binding("a", Dialect::Q3, &context, seen.clone()),
                resources.clone(),
            )
            .unwrap();
        assert_eq!(
            registry
                .bind(
                    command_binding("a", Dialect::Q3, &context, seen.clone()),
                    resources.clone()
                )
                .err()
                .expect("duplicate bind")
                .to_string(),
            "Mod commands are already bound: source/a"
        );
        let mut foreign = command_binding("b", Dialect::Q3, &context, seen.clone());
        foreign.module.id = ProviderId::new("mod", "other");
        assert_eq!(
            registry
                .bind(foreign, resources.clone())
                .err()
                .expect("foreign module")
                .to_string(),
            "Mod commands require their component's module identity"
        );
        let other = IdentityOwner::create("other").unwrap();
        let mut crossed = command_binding("c", Dialect::Q3, &context, seen);
        crossed.cvars.session = other.session().clone();
        assert_eq!(
            registry
                .bind(crossed, resources.clone())
                .err()
                .expect("foreign cvars")
                .to_string(),
            "Mod command cvars belong to another session"
        );
        let staged = ModCommands::new(context.clone(), Rc::new(|| None));
        let pending = ModCommandSource {
            session: context.session.clone(),
            origin: CommandOrigin::ServerConsole,
            producer: None,
        };
        staged.append("deferred\n", &pending, Dialect::Q3).unwrap();
        assert_eq!(
            staged.flush().expect_err("missing buffer").to_string(),
            "Source commands require the candidate's prepared command buffer"
        );
        let (selection, text) = read_mod_command("modcmd source/a status").unwrap();
        assert_eq!(selection.id, "a");
        assert_eq!(text, "status");
        assert_eq!(
            read_mod_command("modcmd").expect_err("usage").to_string(),
            "Usage: modcmd PRODUCT/COMPONENT_ID <command>"
        );
        assert_eq!(
            registry
                .execute(
                    &ModSelection {
                        product: "source".to_string(),
                        id: "missing".to_string()
                    },
                    "status",
                    &context
                )
                .expect_err("missing selection")
                .to_string(),
            "Mod commands are unavailable: source/missing"
        );
        resources.borrow_mut().close().unwrap();
        assert_eq!(logs.borrow().discarded.len(), 1);
    }
}
