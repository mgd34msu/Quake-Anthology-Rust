//! Installed game-content catalog: products, discovery, add-ons, and loadouts.
//!
//! Donor provenance: `src/content/catalog/addons.ts`,
//! `src/content/catalog/equipment.ts`, `src/content/catalog/monsters.ts`,
//! `src/content/catalog/products.ts`, `src/content/catalog/source-program.ts`,
//! `src/content/catalog/start-maps.ts`, `src/content/catalog/timing.ts`,
//! `src/content/catalog/weapon-behavior-document.ts`,
//! `src/content/catalog/weapon-hud.ts`, `src/content/catalog/weapons.ts`,
//! and `src/content/catalog/index.ts` (`~1000` lines combined, plus the
//! two weapon seam files in `catalog/weapon-hud.rs` and
//! `catalog/weapons.rs`). The two `launch.ts` re-export lines in
//! `index.ts` are out of scope and dropped; nothing else in these files
//! touches `launch`.
//!
//! The compat seam extends this module with `src/content/catalog/launch.ts`,
//! `src/content/catalog/weapon-behaviors.ts`,
//! `src/content/catalog/qvm-weapon-behaviors.ts`, and
//! `src/content/catalog/native-weapon-behaviors.ts`. Those donors reach into
//! guest runtimes (`qa-guest`) and source adapters (`qa-compat`), which sit
//! above `qa-content` in the dependency graph, so the seam defines its inputs
//! as caller-provided traits and snapshots (`LaunchWeaponSources`,
//! `LaunchQvmCompatibility`, `QcWeaponProgramSnapshot`,
//! `QvmWeaponBehaviorService`, `NativeWeaponBehaviorService`) instead of
//! calling up the stack. Selected-arsenal adapters (`catalog/weapons.ts`,
//! `catalog/weapon-hud.ts`) live in [`weapons`] and [`weapon_hud`], and
//! [`CatalogWeaponSources`] implements [`LaunchWeaponSources`] with them so
//! launch resolution keeps one seam.
//!
//! The donor is async over Node file handles; this port is synchronous over
//! `std::fs`. [`MountedContent`] drops at scope end, matching the donor's
//! `using` disposal. JSON documents (Quaddicted catalogs, `mapdb.json`,
//! weapon behavior declarations, `.quaddicted.json`) parse through the
//! shared [`SaveJson`] reader.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use qa_core::identity::ProviderId;
use qa_core::numeric::{
    Arithmetic, FloatToInt, NumericProfile, Q1_DONOR_PROFILE, Q2_DONOR_PROFILE, Q3_BINARY32_PROFILE,
};
use qa_core::time::ClockProfile;
use qa_platform::files::contained_file_parts;
use thiserror::Error;

use crate::archive::open_archive;
use crate::contract::{
    create_content_digest, create_content_id, create_mount_id, create_mount_identity, create_mount_plan_id,
    is_content_digest, ArchiveFormat, ArchiveMount, CampaignSelection, CharacterSelection, ContentDigest, ContentId,
    ContentIdentity, ContentMount, ContractError, EnemySelection, EnvironmentSelection, EquipmentSelection,
    ExecutableRecipe, ExecutionModule, ExecutionSelection, FrameOrdering, GameFamily, GrappleBinding,
    GrappleMechanicDetail, GrappleSelection, HandGrenadeSelection, LaunchChoice, LaunchSelection, LazyArchiveDigest,
    LooseMount, MapSelection, ModuleIdentity, ModuleRole, MonsterDefinitionReference, MonsterSelectionTarget, MountId,
    MountPlanId, NativeWeaponBehaviorDeclaration, PrefixMountOrder, PresentationSelection, ProjectileRole,
    ProviderReference, ProviderTiming, Q3ApiIdentity, QvmAbiProfile, QvmGrappleCable, RecipeId,
    ResolvedExecutionModule, ResolvedGameplayMod, ResolvedMap, ResolvedMountPlan, ResolvedResourceReference,
    ResolvedWeaponBehaviorSelection, ResourceProvenance, ResourceRequest, SourceEdition, WeaponBehaviorCallback,
    WeaponBehaviorDefinition, MAX_SAFE_INTEGER,
};
use crate::monsters::{monster_source, monster_timing, provider_text};
use crate::mounts::{
    open_mount_plan, MountError, MountedContent, OpenMountOptions, OpenedResource, OrderedPlan, OrderedReader,
    ResourceRef,
};
use crate::paths::{find_content_path, normalize_resource_path, PathComparison, PathError};
use crate::user_data::user_product_directory;
use crate::value::{parse_save_json, save_error, SaveReader, ValueError};

/// Monster catalog re-exports (donor `catalog/monsters.ts` re-exports).
pub use crate::monsters::{
    campaign_monster_slots, default_monster_roster, monster_sources, MonsterRole, MonsterRosterSlot,
};
/// Parsed JSON document value for catalog JSON inputs.
pub use crate::value::SaveJson;

/// Selected-arsenal HUD icons (donor `catalog/weapon-hud.ts`).
pub mod weapon_hud;
/// Selected-arsenal source adapters (donor `catalog/weapons.ts`).
pub mod weapons;

/// Selected-arsenal seam re-exports.
pub use self::weapon_hud::{weapon_hud_icons, weapon_hud_resources, WeaponHudIcons};
pub use self::weapons::{
    admit_weapon_timing, canonical_weapon_source, q1_hipnotic_weapon_providers, q1_weapon_providers,
    q2_registered_weapon_resources, q2_weapon_providers, selected_weapon_resources, selected_weapon_timing,
    supports_selected_weapon_product, weapon_resources, CatalogWeaponSources, Q1HipnoticWeaponProviders,
    Q1WeaponProviders, Q2WeaponProviders,
};

// `addons.ts` calls `containedFileParts`, whose donor `RangeError` text is
// repeated here so mapping failures keep their donor message.
fn contained(path: &str) -> Result<Vec<String>, CatalogError> {
    contained_file_parts(path).map_err(|_| {
        CatalogError::Invalid("File path must be relative and stay inside its storage directory".to_string())
    })
}

/// Catalog failure (donor `RangeError`/`Error` throws plus wrapped sources).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CatalogError {
    /// Invalid identity, path, JSON, selection, or requirement (donor `RangeError`).
    #[error("{0}")]
    Invalid(String),
    /// Missing content, I/O-independent failure, or state error (donor `Error`).
    #[error("{0}")]
    Failed(String),
    /// Contract failure.
    #[error(transparent)]
    Contract(#[from] ContractError),
    /// Archive failure.
    #[error(transparent)]
    Archive(#[from] crate::archive::ArchiveError),
    /// Mount failure.
    #[error(transparent)]
    Mount(#[from] MountError),
    /// Path failure.
    #[error(transparent)]
    Path(#[from] PathError),
    /// Monster failure.
    #[error(transparent)]
    Monster(#[from] crate::monsters::MonsterError),
    /// JSON value failure.
    #[error(transparent)]
    Value(#[from] crate::value::ValueError),
    /// Filesystem access failed.
    #[error("Catalog I/O error for {path}: {message}")]
    Io {
        /// Requested path.
        path: String,
        /// Underlying error.
        message: String,
    },
}

fn invalid(message: impl Into<String>) -> CatalogError {
    CatalogError::Invalid(message.into())
}

fn failed(message: impl Into<String>) -> CatalogError {
    CatalogError::Failed(message.into())
}

fn io_error(path: &Path, error: std::io::Error) -> CatalogError {
    CatalogError::Io {
        path: path.to_string_lossy().into_owned(),
        message: error.to_string(),
    }
}

/// Whether an I/O error is a plain missing file (`ENOENT` only).
///
/// The user-content scan catches only `ENOENT`, unlike path resolution.
fn is_not_found(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::NotFound
}

/// Build a [`ProviderId`] from donor `"namespace:name"` text.
fn provider_id(text: &str) -> ProviderId {
    match text.split_once(':') {
        Some((namespace, name)) => ProviderId::new(namespace, name),
        None => ProviderId::new("", text),
    }
}

/// Donor game-family text (`"q1"`, `"q2"`, `"q3"`).
#[must_use]
pub fn family_name(family: GameFamily) -> &'static str {
    match family {
        GameFamily::Q1 => "q1",
        GameFamily::Q2 => "q2",
        GameFamily::Q3 => "q3",
    }
}

/// Donor edition text (`"classic"`, `"rerelease"`).
#[must_use]
pub fn edition_name(edition: SourceEdition) -> &'static str {
    match edition {
        SourceEdition::Classic => "classic",
        SourceEdition::Rerelease => "rerelease",
    }
}

/// Parse a source edition (`"classic"`, `"rerelease"`).
fn parse_edition(text: &str) -> Result<SourceEdition, CatalogError> {
    match text {
        "classic" => Ok(SourceEdition::Classic),
        "rerelease" => Ok(SourceEdition::Rerelease),
        _ => Err(invalid(format!("Unknown source edition: {text}"))),
    }
}

/// Lowercase hex encoding of bytes (`Buffer.toString("hex")`).
fn hex_lower(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(DIGITS[(byte >> 4) as usize] as char);
        text.push(DIGITS[(byte & 15) as usize] as char);
    }
    text
}

/// Parent directory of a `/`-separated resource path (`dirname`).
fn posix_dirname(path: &str) -> &str {
    path.rsplit_once('/').map_or(".", |(parent, _)| parent)
}

/// Lexical `..`/`.` normalization for joined paths (donor `resolve`).
fn lexical_join(root: &str, path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    let absolute = root.starts_with('/');
    for part in root.split('/').chain(path.split('/')) {
        if part.is_empty() || part == "." {
            continue;
        } else if part == ".." {
            parts.pop();
        } else {
            parts.push(part);
        }
    }
    let joined = parts.join("/");
    if absolute {
        format!("/{joined}")
    } else {
        joined
    }
}

/// Lexical relative path with `/` separators (donor `relative`).
fn relative_posix(from: &str, to: &str) -> String {
    let from_parts: Vec<&str> = from.split('/').filter(|part| !part.is_empty()).collect();
    let to_parts: Vec<&str> = to.split('/').filter(|part| !part.is_empty()).collect();
    let mut shared = 0;
    while shared < from_parts.len() && shared < to_parts.len() && from_parts[shared] == to_parts[shared] {
        shared += 1;
    }
    let mut parts = vec![".."; from_parts.len() - shared];
    parts.extend_from_slice(&to_parts[shared..]);
    parts.join("/")
}

/// Resolve to an absolute lexically-clean path (donor `resolve`).
fn resolve_path(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    let mut parts: Vec<String> = Vec::new();
    for part in absolute.components() {
        use std::path::Component;
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                let anchored = parts.last().is_none_or(|last| last == "/" || last.ends_with(':'));
                if !anchored {
                    parts.pop();
                }
            }
            other => parts.push(other.as_os_str().to_string_lossy().into_owned()),
        }
    }
    let mut clean = PathBuf::new();
    for part in parts {
        clean.push(part);
    }
    if clean.as_os_str().is_empty() {
        clean.push("/");
    }
    clean
}

/// Lowercased file extension with its dot (`extname().toLowerCase()`).
fn file_extension(name: &str) -> String {
    let base = name.rsplit('/').next().unwrap_or(name);
    match base.rfind('.') {
        Some(dot) if dot > 0 => base[dot..].to_lowercase(),
        _ => String::new(),
    }
}

// `products.ts`.

/// Expected product row from the installed-content inventory (`ProductExpectation`).
///
/// Unavailable rows stay visible so callers can report requirements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductExpectation {
    /// Stable product identity (e.g. `"q1-classic-id1"`).
    pub id: String,
    /// Game family.
    pub family: GameFamily,
    /// Edition (`"classic"`, `"rerelease"`, ...).
    pub edition: String,
    /// Campaign (`"id1"`, `"baseq2"`, ...).
    pub campaign: String,
    /// Display title.
    pub title: String,
    /// Content directory beneath the corpus root.
    pub content_directory: String,
    /// Base product identity, when this product extends another.
    pub base_product: Option<String>,
    /// Archive paths required beneath the corpus root.
    pub required_content_archives: Vec<String>,
    /// Program names supplied by the product.
    pub required_programs: Vec<String>,
    /// Map that must resolve for the product to count as installed.
    pub map_witness: Option<String>,
    /// Reason the product can never resolve, when known statically.
    pub unresolved_reason: Option<String>,
}

type ProductRow = (
    &'static str,
    GameFamily,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    Option<&'static str>,
    &'static [&'static str],
    &'static [&'static str],
    Option<&'static str>,
);

/// Stock product rows (`expectedProducts`).
const EXPECTED_PRODUCT_ROWS: &[ProductRow] = &[
    (
        "q1-classic-id1",
        GameFamily::Q1,
        "classic",
        "id1",
        "Quake",
        "q1/id1",
        None,
        &["q1/id1/pak0.pak", "q1/id1/pak1.pak"],
        &["progs.dat"],
        None,
    ),
    (
        "q1-classic-hipnotic",
        GameFamily::Q1,
        "classic",
        "hipnotic",
        "Scourge of Armagon",
        "q1/hipnotic",
        Some("q1-classic-id1"),
        &["q1/hipnotic/pak0.pak", "q1/id1/pak0.pak", "q1/id1/pak1.pak"],
        &["progs.dat"],
        None,
    ),
    (
        "q1-classic-rogue",
        GameFamily::Q1,
        "classic",
        "rogue",
        "Dissolution of Eternity",
        "q1/rogue",
        Some("q1-classic-id1"),
        &["q1/rogue/pak0.pak", "q1/id1/pak0.pak", "q1/id1/pak1.pak"],
        &["progs.dat"],
        None,
    ),
    (
        "q1-classic-ctf",
        GameFamily::Q1,
        "classic",
        "ctf",
        "Threewave Capture the Flag",
        "q1/ctf",
        Some("q1-classic-id1"),
        &[
            "q1/ctf/pak0.pak",
            "q1/ctf/pak1.pak",
            "q1/id1/pak0.pak",
            "q1/id1/pak1.pak",
        ],
        &["progs.dat"],
        Some("maps/ctfstart.bsp"),
    ),
    (
        "q1-rerelease-id1",
        GameFamily::Q1,
        "rerelease",
        "id1",
        "Quake",
        "q1/rerelease/id1",
        None,
        &["q1/rerelease/id1/pak0.pak"],
        &["progs.dat"],
        None,
    ),
    (
        "q1-rerelease-hipnotic",
        GameFamily::Q1,
        "rerelease",
        "hipnotic",
        "Scourge of Armagon",
        "q1/rerelease/hipnotic",
        Some("q1-rerelease-id1"),
        &["q1/rerelease/hipnotic/pak0.pak", "q1/rerelease/id1/pak0.pak"],
        &["progs.dat"],
        None,
    ),
    (
        "q1-rerelease-rogue",
        GameFamily::Q1,
        "rerelease",
        "rogue",
        "Dissolution of Eternity",
        "q1/rerelease/rogue",
        Some("q1-rerelease-id1"),
        &["q1/rerelease/rogue/pak0.pak", "q1/rerelease/id1/pak0.pak"],
        &["progs.dat"],
        None,
    ),
    (
        "q1-rerelease-dopa",
        GameFamily::Q1,
        "rerelease",
        "dopa",
        "Dimension of the Past",
        "q1/rerelease/dopa",
        Some("q1-rerelease-id1"),
        &["q1/rerelease/dopa/pak0.pak", "q1/rerelease/id1/pak0.pak"],
        &["progs.dat"],
        None,
    ),
    (
        "q1-rerelease-mg1",
        GameFamily::Q1,
        "rerelease",
        "mg1",
        "Dimension of the Machine",
        "q1/rerelease/mg1",
        Some("q1-rerelease-id1"),
        &["q1/rerelease/mg1/pak0.pak", "q1/rerelease/id1/pak0.pak"],
        &["progs.dat"],
        None,
    ),
    (
        "q1-rerelease-mg3",
        GameFamily::Q1,
        "rerelease",
        "mg3",
        "Dawn of the Machine",
        "q1/rerelease/mg3",
        Some("q1-rerelease-id1"),
        &["q1/rerelease/mg3/pak0.pak", "q1/rerelease/id1/pak0.pak"],
        &["progs.dat"],
        None,
    ),
    (
        "q1-rerelease-ctf",
        GameFamily::Q1,
        "rerelease",
        "ctf",
        "Capture the Flag",
        "q1/rerelease/ctf",
        Some("q1-rerelease-id1"),
        &["q1/rerelease/ctf/pak0.pak", "q1/rerelease/id1/pak0.pak"],
        &["progs.dat"],
        None,
    ),
    (
        "q1-quakeworld",
        GameFamily::Q1,
        "quakeworld",
        "id1",
        "QuakeWorld",
        "q1/qw",
        Some("q1-classic-id1"),
        &["q1/id1/pak0.pak", "q1/id1/pak1.pak"],
        &["qwprogs.dat"],
        None,
    ),
    (
        "q1-rerelease-quake64",
        GameFamily::Q1,
        "rerelease",
        "quake64",
        "Quake 64",
        "q1/rerelease/q64",
        Some("q1-rerelease-id1"),
        &["q1/rerelease/id1/pak0.pak"],
        &["progs.dat"],
        Some("maps/start.bsp"),
    ),
    (
        "q2-classic-baseq2",
        GameFamily::Q2,
        "classic",
        "baseq2",
        "Quake II",
        "q2/baseq2",
        None,
        &["q2/baseq2/pak0.pak"],
        &["gamex86.dll"],
        Some("maps/base1.bsp"),
    ),
    (
        "q2-classic-xatrix",
        GameFamily::Q2,
        "classic",
        "xatrix",
        "The Reckoning",
        "q2/xatrix",
        Some("q2-classic-baseq2"),
        &["q2/xatrix/pak0.pak", "q2/baseq2/pak0.pak"],
        &["gamex86.dll"],
        Some("maps/xswamp.bsp"),
    ),
    (
        "q2-classic-rogue",
        GameFamily::Q2,
        "classic",
        "rogue",
        "Ground Zero",
        "q2/rogue",
        Some("q2-classic-baseq2"),
        &["q2/rogue/pak0.pak", "q2/baseq2/pak0.pak"],
        &["gamex86.dll"],
        Some("maps/rmine1.bsp"),
    ),
    (
        "q2-classic-ctf",
        GameFamily::Q2,
        "classic",
        "ctf",
        "Capture the Flag",
        "q2/ctf",
        Some("q2-classic-baseq2"),
        &["q2/ctf/pak0.pak", "q2/baseq2/pak0.pak"],
        &["gamex86.dll"],
        Some("maps/q2ctf1.bsp"),
    ),
    (
        "q2-classic-lmctf",
        GameFamily::Q2,
        "classic",
        "lmctf",
        "Loki's Minions CTF",
        "q2/lmctf",
        Some("q2-classic-baseq2"),
        &["q2/lmctf/pak0.pak", "q2/baseq2/pak0.pak"],
        &["gamex86.dll"],
        Some("maps/lmctf09.bsp"),
    ),
    (
        "q2-rerelease-baseq2",
        GameFamily::Q2,
        "rerelease",
        "baseq2",
        "Quake II",
        "q2/rerelease/baseq2",
        None,
        &["q2/rerelease/baseq2/pak0.pak"],
        &["game_x64.dll"],
        Some("maps/base1.bsp"),
    ),
    (
        "q2-rerelease-xatrix",
        GameFamily::Q2,
        "rerelease",
        "xatrix",
        "The Reckoning",
        "q2/rerelease/baseq2",
        Some("q2-rerelease-baseq2"),
        &["q2/rerelease/baseq2/pak0.pak"],
        &["game_x64.dll"],
        Some("maps/xswamp.bsp"),
    ),
    (
        "q2-rerelease-rogue",
        GameFamily::Q2,
        "rerelease",
        "rogue",
        "Ground Zero",
        "q2/rerelease/baseq2",
        Some("q2-rerelease-baseq2"),
        &["q2/rerelease/baseq2/pak0.pak"],
        &["game_x64.dll"],
        Some("maps/rmine1.bsp"),
    ),
    (
        "q2-rerelease-ctf",
        GameFamily::Q2,
        "rerelease",
        "ctf",
        "Capture the Flag",
        "q2/rerelease/baseq2",
        Some("q2-rerelease-baseq2"),
        &["q2/rerelease/baseq2/pak0.pak"],
        &["game_x64.dll"],
        Some("maps/q2ctf1.bsp"),
    ),
    (
        "q2-rerelease-mg2",
        GameFamily::Q2,
        "rerelease",
        "mg2",
        "Call of the Machine",
        "q2/rerelease/baseq2",
        Some("q2-rerelease-baseq2"),
        &["q2/rerelease/baseq2/pak0.pak"],
        &["game_x64.dll"],
        Some("maps/mguhub.bsp"),
    ),
    (
        "q2-rerelease-n64",
        GameFamily::Q2,
        "rerelease",
        "n64",
        "Quake II 64",
        "q2/rerelease/baseq2",
        Some("q2-rerelease-baseq2"),
        &["q2/rerelease/baseq2/pak0.pak"],
        &["game_x64.dll"],
        Some("maps/q64/rtest.bsp"),
    ),
    (
        "q3-baseq3",
        GameFamily::Q3,
        "classic",
        "baseq3",
        "Quake III Arena",
        "q3a/baseq3",
        None,
        &["q3a/baseq3/pak0.pk3"],
        &["vm/qagame.qvm", "vm/cgame.qvm", "vm/ui.qvm"],
        None,
    ),
    (
        "q3-missionpack",
        GameFamily::Q3,
        "classic",
        "missionpack",
        "Team Arena",
        "q3a/missionpack",
        Some("q3-baseq3"),
        &["q3a/missionpack/pak0.pk3", "q3a/baseq3/pak0.pk3"],
        &["vm/qagame.qvm", "vm/cgame.qvm", "vm/ui.qvm"],
        None,
    ),
    (
        "q3-demota",
        GameFamily::Q3,
        "demo",
        "demota",
        "Quake III restricted demo content",
        "q3a/demota",
        None,
        &["q3a/demota/pak0.pk3"],
        &["vm/qagame.qvm", "vm/cgame.qvm", "vm/ui.qvm"],
        None,
    ),
];

/// Stock product rows (`expectedProducts`).
#[must_use]
pub fn expected_products() -> Vec<ProductExpectation> {
    EXPECTED_PRODUCT_ROWS
        .iter()
        .map(
            |(
                id,
                family,
                edition,
                campaign,
                title,
                content_directory,
                base_product,
                required_content_archives,
                required_programs,
                map_witness,
            )| ProductExpectation {
                id: (*id).to_string(),
                family: *family,
                edition: (*edition).to_string(),
                campaign: (*campaign).to_string(),
                title: (*title).to_string(),
                content_directory: (*content_directory).to_string(),
                base_product: base_product.map(str::to_string),
                required_content_archives: required_content_archives.iter().map(|name| name.to_string()).collect(),
                required_programs: required_programs.iter().map(|name| name.to_string()).collect(),
                map_witness: map_witness.map(str::to_string),
                unresolved_reason: None,
            },
        )
        .collect()
}

// `timing.ts`.

/// Native numeric plus clock profile for a provider (`nativeProviderTiming`).
#[must_use]
pub fn native_provider_timing(provider: &ProviderReference, family: GameFamily, rerelease: bool) -> ProviderTiming {
    let id = match family {
        GameFamily::Q1 => "q1:binary32",
        GameFamily::Q2 => "q2:binary32",
        GameFamily::Q3 => "q3:binary32",
    };
    ProviderTiming {
        provider: provider.provider.clone(),
        clock: match family {
            GameFamily::Q1 => ClockProfile::Q1Netquake {
                minimum_frame_seconds: 0.001,
                maximum_frame_seconds: 0.1,
                fixed_frame_seconds: None,
            },
            GameFamily::Q2 if rerelease => ClockProfile::Q2Rerelease {
                frame_milliseconds: 25.0,
            },
            GameFamily::Q2 => ClockProfile::Q2Classic,
            GameFamily::Q3 => ClockProfile::Q3 {
                server_frame_milliseconds: 50.0,
                fixed_movement_milliseconds: None,
            },
        },
        numeric: NumericProfile {
            id,
            arithmetic: Arithmetic::Binary32EachOp,
            float_to_int: FloatToInt::CheckedTruncation,
        },
    }
}

// `weapon-behavior-document.ts`.

/// Parsed weapon behavior declaration (`WeaponBehaviorDocument`).
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponBehaviorDocument {
    /// Declaration version, always `1`.
    pub version: u32,
    /// Behavior artifact path, when declared.
    pub artifact_path: Option<String>,
    /// Behavior entries (uninterpreted JSON).
    pub behaviors: Vec<SaveJson>,
}

/// Read a weapon behavior declaration (`readWeaponBehaviorDocument`).
pub fn read_weapon_behavior_document(bytes: &[u8]) -> Result<WeaponBehaviorDocument, CatalogError> {
    let text = std::str::from_utf8(bytes).map_err(|_| invalid("Invalid weapon behavior declaration"))?;
    let value = parse_save_json(text).map_err(|_| invalid("Invalid weapon behavior declaration"))?;
    let SaveJson::Object(_) = value else {
        return Err(invalid("Invalid weapon behavior declaration"));
    };
    let version_ok = matches!(value.get("version"), Some(SaveJson::Number(version)) if *version == 1.0);
    let behaviors = match value.get("behaviors") {
        Some(SaveJson::Array(behaviors)) => behaviors.clone(),
        _ => return Err(invalid("Invalid weapon behavior declaration")),
    };
    if !version_ok {
        return Err(invalid("Invalid weapon behavior declaration"));
    }
    let artifact = match value.get("artifactPath") {
        None => None,
        Some(SaveJson::String(artifact)) => Some(normalize_resource_path(artifact)?),
        Some(_) => return Err(invalid("Invalid weapon behavior artifact path")),
    };
    Ok(WeaponBehaviorDocument {
        version: 1,
        artifact_path: artifact,
        behaviors,
    })
}

/// Read a behavior entry identity (`weaponBehaviorEntryId`).
pub fn weapon_behavior_entry_id(entry: &SaveJson) -> Result<String, CatalogError> {
    match entry {
        SaveJson::Object(_) => match entry.get("id") {
            Some(SaveJson::String(id)) => Ok(id.clone()),
            _ => Err(invalid("Invalid weapon behavior entry")),
        },
        _ => Err(invalid("Invalid weapon behavior entry")),
    }
}

// `start-maps.ts`.

/// Authored episode row (`AuthoredEpisode`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoredEpisode {
    /// Episode identity.
    pub id: String,
    /// Console command.
    pub command: String,
    /// Display name.
    pub name: String,
    /// Activity text.
    pub activity: String,
    /// Whether skill selection precedes the episode.
    pub needs_skill_select: bool,
}

/// Authored start-map row (`AuthoredStartMap`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoredStartMap {
    /// Owning episode.
    pub episode: String,
    /// Authored BSP chain (donor `MapDB_ResolveBsp` keeps the cinematic chain).
    pub bsp: String,
    /// Resolved `maps/<name>.bsp` path of the final BSP.
    pub path: String,
    /// Display title.
    pub title: String,
    /// Starting inventory.
    pub start_items: String,
    /// Singleplayer start.
    pub singleplayer: bool,
    /// Cooperative start.
    pub cooperative: bool,
    /// Capture-the-flag start.
    pub capture_the_flag: bool,
}

/// Parsed episode plus starts without the resolved resource (`parseAuthoredStarts` result).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoredStarts {
    /// Matching episode, when the database names one.
    pub episode: Option<AuthoredEpisode>,
    /// Matching starts.
    pub starts: Vec<AuthoredStartMap>,
}

/// Resolved authored starts (`AuthoredStartCatalog`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoredStartCatalog {
    /// Resolved `mapdb.json` reference.
    pub resource: ResolvedResourceReference,
    /// Matching episode, when the database names one.
    pub episode: Option<AuthoredEpisode>,
    /// Matching starts.
    pub starts: Vec<AuthoredStartMap>,
}

fn mapdb_record(value: &SaveJson) -> Result<(), CatalogError> {
    match value {
        SaveJson::Object(_) => Ok(()),
        _ => Err(invalid("Invalid mapdb.json object")),
    }
}

fn mapdb_string(reader: &SaveReader) -> Result<String, CatalogError> {
    if reader.is_missing() {
        return Ok(String::new());
    }
    reader.string().map_err(|_| invalid("Invalid mapdb.json string"))
}

fn mapdb_flag(reader: &SaveReader) -> Result<bool, CatalogError> {
    if reader.is_missing() {
        return Ok(false);
    }
    reader.boolean().map_err(|_| invalid("Invalid mapdb.json flag"))
}

fn mapdb_entries(reader: &SaveReader) -> Result<Vec<SaveJson>, CatalogError> {
    if reader.is_missing() {
        return Ok(Vec::new());
    }
    reader
        .list(|item| Ok::<SaveJson, CatalogError>(item.value.cloned().unwrap_or(SaveJson::Null)))
        .map_err(|_| invalid("Invalid mapdb.json array"))
}

/// Parse authored starts; the donor `MapDB_ResolveBsp` selects the final BSP
/// while retaining the authored cinematic chain (`parseAuthoredStarts`).
///
/// Bytes decode lossily: the donor uses a non-fatal `TextDecoder` here.
pub fn parse_authored_starts(bytes: &[u8], campaign: &str) -> Result<AuthoredStarts, CatalogError> {
    let text = String::from_utf8_lossy(bytes);
    let value = parse_save_json(&text).map_err(|_| invalid("Invalid mapdb.json object"))?;
    mapdb_record(&value)?;
    let database = SaveReader::new(&value);
    let episode_id = if campaign == "ctf" { "baseq2" } else { campaign };
    let mut episode = None;
    for row in mapdb_entries(&database.field("episodes"))? {
        mapdb_record(&row)?;
        let row = SaveReader::new(&row);
        if mapdb_string(&row.field("id"))? != episode_id {
            continue;
        }
        episode = Some(AuthoredEpisode {
            id: mapdb_string(&row.field("id"))?,
            command: mapdb_string(&row.field("command"))?,
            name: mapdb_string(&row.field("name"))?,
            activity: mapdb_string(&row.field("activity"))?,
            needs_skill_select: mapdb_flag(&row.field("needsSkillSelect"))?,
        });
        break;
    }
    let mut starts = Vec::new();
    for row in mapdb_entries(&database.field("maps"))? {
        mapdb_record(&row)?;
        let row = SaveReader::new(&row);
        if mapdb_string(&row.field("episode"))? != episode_id {
            continue;
        }
        let selected = if campaign == "ctf" {
            mapdb_flag(&row.field("ctf"))?
        } else {
            mapdb_flag(&row.field("sp"))?
        };
        if !selected {
            continue;
        }
        let bsp = mapdb_string(&row.field("bsp"))?;
        let last = bsp.rsplit('+').next().unwrap_or("");
        let name = last.strip_prefix('*').unwrap_or(last);
        if name.is_empty()
            || name
                .chars()
                .any(|c| matches!(c, '+' | '*' | '$' | ';') || c.is_whitespace())
        {
            return Err(invalid(format!("Invalid mapdb.json start BSP: {bsp}")));
        }
        let path = normalize_resource_path(&format!("maps/{name}.bsp"))?;
        starts.push(AuthoredStartMap {
            episode: episode_id.to_string(),
            bsp,
            path,
            title: mapdb_string(&row.field("title"))?,
            start_items: mapdb_string(&row.field("start_items"))?,
            singleplayer: mapdb_flag(&row.field("sp"))?,
            cooperative: mapdb_flag(&row.field("coop"))?,
            capture_the_flag: mapdb_flag(&row.field("ctf"))?,
        });
    }
    Ok(AuthoredStarts { episode, starts })
}

// `source-program.ts`.

/// Follow program-less mod links to the product supplying the source program
/// (`sourceProgramProduct`).
pub fn source_program_product<'a>(
    catalog: &'a InstalledCatalog,
    content: &str,
) -> Result<&'a CatalogProduct, CatalogError> {
    let builtin: HashSet<&str> = EXPECTED_PRODUCT_ROWS.iter().map(|row| row.0).collect();
    let mut product = catalog.require(content)?;
    let mut visited: HashSet<&str> = HashSet::new();
    while !builtin.contains(product.expectation.id.as_str())
        && product.expectation.required_programs.is_empty()
        && product.expectation.base_product.is_some()
    {
        if !visited.insert(product.id.as_str()) {
            return Err(failed("Cyclic source program dependency"));
        }
        let base_id = product.expectation.base_product.clone().unwrap_or_default();
        let base = catalog.require(&base_id)?;
        if base.expectation.family != product.expectation.family
            || base.expectation.edition != product.expectation.edition
        {
            return Err(failed("Source program dependency changes game family or edition"));
        }
        product = base;
    }
    Ok(product)
}

/// Source implementation identity for a product (`sourceProgramImplementation`).
#[must_use]
pub fn source_program_implementation(product: &ProductExpectation) -> ProviderId {
    ProviderId::new(
        family_name(product.family),
        &format!("source/{}/{}", product.edition, product.campaign),
    )
}

fn execution_owner<Artifact>(module: &ExecutionModule<Artifact>) -> &ProviderReference {
    match module {
        ExecutionModule::Builtin { owner, .. }
        | ExecutionModule::Quakec { owner, .. }
        | ExecutionModule::Qvm { owner, .. }
        | ExecutionModule::Native { owner, .. } => owner,
    }
}

fn execution_role<Artifact>(module: &ExecutionModule<Artifact>) -> ModuleRole {
    match module {
        ExecutionModule::Builtin { role, .. }
        | ExecutionModule::Qvm { role, .. }
        | ExecutionModule::Native { role, .. } => *role,
        ExecutionModule::Quakec { .. } => ModuleRole::ServerGame,
    }
}

/// Campaign supplying the recipe's server game (`selectedSourceProgram`).
///
/// Returns the owner package for native modules and the mapped stock
/// campaign for replacement TypeScript implementations.
pub fn selected_source_program<Artifact>(
    execution: &[ExecutionModule<Artifact>],
    entities: &ProviderReference,
) -> Result<Option<String>, CatalogError> {
    let module = execution.iter().find(|module| {
        execution_role(module) == ModuleRole::ServerGame
            && execution_owner(module).provider == entities.provider
            && execution_owner(module).content == entities.content
    });
    let replacement = match module {
        Some(ExecutionModule::Builtin { implementation, .. }) => Some(implementation),
        _ => None,
    };
    match replacement {
        Some(implementation) if *implementation != entities.provider => {
            let mut parts = entities.content.as_str().split(':');
            let family = parts.next().unwrap_or("");
            let edition = parts.next().unwrap_or("");
            let product = expected_products().into_iter().find(|product| {
                family_name(product.family) == family
                    && product.edition == edition
                    && source_program_implementation(product) == *implementation
            });
            match product {
                Some(product) => Ok(Some(product.campaign)),
                None => Err(failed(format!(
                    "Unsupported source implementation {}",
                    provider_text(implementation)
                ))),
            }
        }
        _ => Ok(entities.content.as_str().split(':').nth(2).map(str::to_string)),
    }
}

// `monsters.ts` (donor `monsterSources`, `campaignMonsterSlots`,
// `defaultMonsterRoster`, `MonsterRole`, and `MonsterRosterSlot` are
// re-exported at the top of this module).

/// Monster definitions selected by an enemy selection (`selectedMonsterDefinitions`).
///
/// The default leads; per-classname overrides follow in classname order so
/// output stays deterministic.
#[must_use]
pub fn selected_monster_definitions(enemies: &EnemySelection) -> Vec<MonsterDefinitionReference> {
    match enemies {
        EnemySelection::MapDefined => Vec::new(),
        EnemySelection::Replace { default, by_classname } => {
            let mut definitions = Vec::new();
            if let MonsterSelectionTarget::Defined(definition) = default {
                definitions.push(definition.clone());
            }
            let mut classnames: Vec<&String> = by_classname.keys().collect();
            classnames.sort();
            for classname in classnames {
                if let Some(MonsterSelectionTarget::Defined(definition)) = by_classname.get(classname) {
                    definitions.push(definition.clone());
                }
            }
            definitions
        }
    }
}

/// Validate monster definitions against installed content (`validateMonsters`).
pub fn validate_monsters(enemies: &EnemySelection, catalog: &InstalledCatalog) -> Result<(), CatalogError> {
    for definition in selected_monster_definitions(enemies) {
        let source = monster_source(&definition)?;
        let product = catalog.require(definition.source.content.as_str())?.expectation.clone();
        let edition = match source.edition {
            SourceEdition::Classic => "classic",
            SourceEdition::Rerelease => "rerelease",
        };
        if family_name(product.family) != source.family.as_str()
            || product.edition != edition
            || product.campaign != source.program.as_str()
        {
            return Err(invalid(format!(
                "Monster source content does not match {}: {}",
                provider_text(&definition.source.provider),
                definition.source.content.as_str()
            )));
        }
    }
    if let EnemySelection::Replace { by_classname, .. } = enemies {
        if by_classname.keys().any(String::is_empty) {
            return Err(invalid("An authored monster classname cannot be empty"));
        }
    }
    Ok(())
}

/// Timing for the selected monster providers (`selectedMonsterTiming`).
#[must_use]
pub fn selected_monster_timing(enemies: &EnemySelection) -> Vec<ProviderTiming> {
    let providers: HashSet<ProviderId> = selected_monster_definitions(enemies)
        .iter()
        .map(|definition| definition.source.provider.clone())
        .collect();
    monster_sources()
        .into_iter()
        .filter(|source| providers.contains(&source.provider))
        .map(|source| monster_timing(&source))
        .collect()
}

/// Resource requests for the selected monster definitions (`monsterResources`).
pub fn monster_resources(enemies: &EnemySelection) -> Result<Vec<ResourceRequest>, CatalogError> {
    let mut requests = Vec::new();
    for definition in selected_monster_definitions(enemies) {
        let source = monster_source(&definition)?;
        let creature = source.creatures.get(&definition.classname).ok_or_else(|| {
            invalid(format!(
                "Missing registered monster resources: {}",
                definition.classname
            ))
        })?;
        for path in &creature.resources {
            requests.push(ResourceRequest {
                content: definition.source.content.clone(),
                path: path.clone(),
            });
        }
    }
    Ok(requests)
}

// `equipment.ts`.

/// Reusable source equipment provider identities (`EQUIPMENT_PROVIDERS`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EquipmentProviders {
    /// Quake Threewave grapple provider.
    pub threewave: ProviderId,
    /// Quake II CTF grapple provider.
    pub ctf: ProviderId,
    /// Quake II LMCTF grapple provider.
    pub lmctf: ProviderId,
    /// Quake II hand grenade provider.
    pub hand_grenades: ProviderId,
}

/// Reusable source equipment provider identities (`EQUIPMENT_PROVIDERS`).
#[must_use]
pub fn equipment_providers() -> EquipmentProviders {
    EquipmentProviders {
        threewave: provider_id("q1:equipment/threewave-grapple"),
        ctf: provider_id("q2:equipment/ctf-grapple"),
        lmctf: provider_id("q2:equipment/lmctf-grapple"),
        hand_grenades: provider_id("q2:equipment/hand-grenades"),
    }
}

/// Fully disabled equipment (`disabledEquipment`).
#[must_use]
pub fn disabled_equipment() -> EquipmentSelection {
    EquipmentSelection {
        grapple: GrappleSelection::Disabled,
        hand_grenades: HandGrenadeSelection::Disabled,
    }
}

/// Selectable grapple style (`GrappleStyle`).
#[derive(Debug, Clone, PartialEq)]
pub struct GrappleStyle {
    /// Mechanic identity (`"q1-threewave"`, `"q2-ctf"`, `"q2-lmctf"`).
    pub id: String,
    /// Display title.
    pub title: String,
    /// Enabled grapple selection.
    pub selection: GrappleSelection,
    /// Reason the style is unavailable, when it is.
    pub unavailable: Option<String>,
}

fn equipment_unavailable(product: &CatalogProduct) -> Option<String> {
    match &product.availability {
        ProductAvailability::Installed => None,
        ProductAvailability::Unresolved { reason } => Some(reason.clone()),
        ProductAvailability::Missing { .. } => Some(format!("Requires {} game files", product.expectation.title)),
    }
}

fn grapple_for_product(product: &CatalogProduct) -> Option<GrappleSelection> {
    let providers = equipment_providers();
    let expectation = &product.expectation;
    let source = |provider: ProviderId| ProviderReference {
        provider,
        content: product.id.clone(),
    };
    if expectation.family == GameFamily::Q1 && expectation.campaign == "ctf" {
        let edition = parse_edition(&expectation.edition).ok()?;
        return Some(GrappleSelection::Enabled {
            source: source(providers.threewave),
            binding: GrappleBinding::Slot,
            mechanic: GrappleMechanicDetail::Q1Threewave { edition },
        });
    }
    if expectation.family == GameFamily::Q2 && expectation.campaign == "ctf" {
        let edition = parse_edition(&expectation.edition).ok()?;
        return Some(GrappleSelection::Enabled {
            source: source(providers.ctf),
            binding: GrappleBinding::Slot,
            mechanic: GrappleMechanicDetail::Q2Ctf { edition },
        });
    }
    if expectation.family == GameFamily::Q2 && expectation.campaign == "lmctf" && expectation.edition == "classic" {
        return Some(GrappleSelection::Enabled {
            source: source(providers.lmctf),
            binding: GrappleBinding::Offhand,
            mechanic: GrappleMechanicDetail::Q2Lmctf,
        });
    }
    None
}

fn grapple_style_name(mechanic: &GrappleMechanicDetail) -> Option<(&'static str, &'static str)> {
    match mechanic {
        GrappleMechanicDetail::Q1Threewave { .. } => Some(("q1-threewave", "Threewave CTF (Quake 1)")),
        GrappleMechanicDetail::Q2Ctf { .. } => Some(("q2-ctf", "Threewave CTF (Quake 2)")),
        GrappleMechanicDetail::Q2Lmctf => Some(("q2-lmctf", "LMCTF (Quake 2)")),
        GrappleMechanicDetail::Q3Qvm { .. } => None,
    }
}

/// Grapple styles from installed CTF/LMCTF sources (`grappleStyles`).
///
/// The registry contains actual hook implementations, independent of
/// slot/offhand placement.
#[must_use]
pub fn grapple_styles(catalog: &InstalledCatalog, preferred: &CatalogProduct) -> Vec<GrappleStyle> {
    let mut sources: Vec<&CatalogProduct> = catalog
        .products
        .iter()
        .filter(|product| product.expectation.campaign == "ctf" || product.expectation.campaign == "lmctf")
        .collect();
    sources.sort_by(|left, right| {
        let installed = |product: &CatalogProduct| product.availability == ProductAvailability::Installed;
        installed(right).cmp(&installed(left)).then_with(|| {
            (right.expectation.edition == preferred.expectation.edition)
                .cmp(&(left.expectation.edition == preferred.expectation.edition))
        })
    });
    let mut styles: Vec<GrappleStyle> = Vec::new();
    for product in sources {
        let Some(selection) = grapple_for_product(product) else {
            continue;
        };
        let GrappleSelection::Enabled { mechanic, .. } = &selection else {
            continue;
        };
        // The curated Threewave style uses its original Morning Star.
        // Rerelease packages retain their different model when selected by
        // a native game or save.
        if matches!(
            mechanic,
            GrappleMechanicDetail::Q1Threewave { edition } if *edition != SourceEdition::Classic
        ) {
            continue;
        }
        let Some((id, title)) = grapple_style_name(mechanic) else {
            continue;
        };
        if styles.iter().any(|style| style.id == id) {
            continue;
        }
        styles.push(GrappleStyle {
            id: id.to_string(),
            title: title.to_string(),
            selection,
            unavailable: equipment_unavailable(product),
        });
    }
    styles
}

/// Installed Quake II base game supplying offhand grenades (`offhandGrenadeSource`).
#[must_use]
pub fn offhand_grenade_source<'a>(
    catalog: &'a InstalledCatalog,
    preferred: &CatalogProduct,
) -> Option<&'a CatalogProduct> {
    let mut sources: Vec<&CatalogProduct> = catalog
        .products
        .iter()
        .filter(|product| {
            product.expectation.family == GameFamily::Q2
                && product.expectation.campaign == "baseq2"
                && product.availability == ProductAvailability::Installed
                && (product.expectation.edition == "classic" || product.expectation.edition == "rerelease")
        })
        .collect();
    sources.sort_by(|left, right| {
        (right.expectation.edition == preferred.expectation.edition)
            .cmp(&(left.expectation.edition == preferred.expectation.edition))
    });
    sources.into_iter().next()
}

/// Equipment native to a map/match pair (`nativeEquipment`).
pub fn native_equipment(
    catalog: &InstalledCatalog,
    map: &ProviderReference,
    match_reference: &ProviderReference,
) -> Result<EquipmentSelection, CatalogError> {
    let product = catalog.require(map.content.as_str())?;
    let rules = catalog.require(match_reference.content.as_str())?;
    if let Some(
        native @ GrappleSelection::Enabled {
            mechanic: GrappleMechanicDetail::Q1Threewave { .. },
            ..
        },
    ) = grapple_for_product(product)
    {
        return Ok(EquipmentSelection {
            grapple: native,
            hand_grenades: HandGrenadeSelection::Disabled,
        });
    }
    let ctf = provider_id("q2:ctf");
    let lmctf = provider_id("q2:lmctf");
    let source = if match_reference.provider == ctf || match_reference.provider == lmctf {
        rules
    } else {
        product
    };
    Ok(EquipmentSelection {
        grapple: grapple_for_product(source).unwrap_or(GrappleSelection::Disabled),
        hand_grenades: HandGrenadeSelection::Disabled,
    })
}

/// Providers referenced by an equipment selection (`equipmentProviders`).
#[must_use]
pub fn equipment_provider_refs(equipment: &EquipmentSelection) -> Vec<ProviderReference> {
    let mut providers = Vec::new();
    if let GrappleSelection::Enabled { source, .. } = &equipment.grapple {
        providers.push(source.clone());
    }
    if let HandGrenadeSelection::Enabled { source, .. } = &equipment.hand_grenades {
        providers.push(source.clone());
    }
    providers
}

/// Validate equipment sources against installed content (`validateEquipment`).
pub fn validate_equipment(equipment: &EquipmentSelection, catalog: &InstalledCatalog) -> Result<(), CatalogError> {
    let providers = equipment_providers();
    let check = |source: &ProviderReference,
                 provider: &ProviderId,
                 family: GameFamily,
                 campaign: &str,
                 edition: SourceEdition|
     -> Result<(), CatalogError> {
        let product = catalog.require(source.content.as_str())?.expectation.clone();
        if source.provider != *provider
            || product.family != family
            || product.campaign != campaign
            || product.edition != edition_name(edition)
        {
            return Err(invalid(format!(
                "Equipment source {}/{} does not supply {} {}",
                provider_text(&source.provider),
                source.content.as_str(),
                provider_text(provider),
                edition_name(edition)
            )));
        }
        Ok(())
    };
    if let GrappleSelection::Enabled { source, mechanic, .. } = &equipment.grapple {
        match mechanic {
            GrappleMechanicDetail::Q1Threewave { edition } => {
                check(source, &providers.threewave, GameFamily::Q1, "ctf", *edition)?;
            }
            GrappleMechanicDetail::Q2Ctf { edition } => {
                check(source, &providers.ctf, GameFamily::Q2, "ctf", *edition)?;
            }
            GrappleMechanicDetail::Q2Lmctf => {
                check(
                    source,
                    &providers.lmctf,
                    GameFamily::Q2,
                    "lmctf",
                    SourceEdition::Classic,
                )?;
            }
            GrappleMechanicDetail::Q3Qvm { profile } => {
                let source_product = catalog.require(source.content.as_str())?.expectation.clone();
                if source_product.family != GameFamily::Q3 || source.provider != profile.module.id {
                    return Err(invalid("QVM hook source differs from its declared executable"));
                }
            }
        }
    }
    if let HandGrenadeSelection::Enabled {
        source,
        edition,
        initial_ammo,
        capacity,
        ..
    } = &equipment.hand_grenades
    {
        check(source, &providers.hand_grenades, GameFamily::Q2, "baseq2", *edition)?;
        let whole = |value: f64| value.trunc() == value && value.abs() <= MAX_SAFE_INTEGER as f64;
        if !whole(*initial_ammo) || !whole(*capacity) || *initial_ammo < 0.0 || *capacity < *initial_ammo {
            return Err(invalid(
                "Equipment hand grenade allowance must be whole ammunition within capacity",
            ));
        }
    }
    Ok(())
}

fn push_equipment_timing(
    timings: &mut Vec<ProviderTiming>,
    source: &ProviderReference,
    edition: SourceEdition,
    q1: bool,
) {
    timings.push(ProviderTiming {
        provider: source.provider.clone(),
        clock: if q1 {
            ClockProfile::Q1Netquake {
                minimum_frame_seconds: 0.001,
                maximum_frame_seconds: 0.1,
                fixed_frame_seconds: None,
            }
        } else if edition == SourceEdition::Classic {
            ClockProfile::Q2Classic
        } else {
            ClockProfile::Q2Rerelease {
                frame_milliseconds: 25.0,
            }
        },
        numeric: if q1 { Q1_DONOR_PROFILE } else { Q2_DONOR_PROFILE },
    });
}

/// Timing for the selected equipment providers (`equipmentTiming`).
#[must_use]
pub fn equipment_timing(equipment: &EquipmentSelection) -> Vec<ProviderTiming> {
    let mut timings = Vec::new();
    if let GrappleSelection::Enabled { source, mechanic, .. } = &equipment.grapple {
        match mechanic {
            GrappleMechanicDetail::Q3Qvm { .. } => timings.push(ProviderTiming {
                provider: source.provider.clone(),
                clock: ClockProfile::Q3 {
                    server_frame_milliseconds: 50.0,
                    fixed_movement_milliseconds: None,
                },
                numeric: Q3_BINARY32_PROFILE,
            }),
            GrappleMechanicDetail::Q1Threewave { edition } => {
                push_equipment_timing(&mut timings, source, *edition, true);
            }
            GrappleMechanicDetail::Q2Ctf { edition } => {
                push_equipment_timing(&mut timings, source, *edition, false);
            }
            GrappleMechanicDetail::Q2Lmctf => {
                push_equipment_timing(&mut timings, source, SourceEdition::Classic, false);
            }
        }
    }
    if let HandGrenadeSelection::Enabled { source, edition, .. } = &equipment.hand_grenades {
        push_equipment_timing(&mut timings, source, *edition, false);
    }
    timings
}

/// Resource requests for the selected equipment (`equipmentResources`).
#[must_use]
pub fn equipment_resources(equipment: &EquipmentSelection) -> Vec<ResourceRequest> {
    let mut requests: Vec<ResourceRequest> = Vec::new();
    let mut add = |source: &ProviderReference, paths: Vec<String>| {
        for path in paths {
            requests.push(ResourceRequest {
                content: source.content.clone(),
                path,
            });
        }
    };
    if let GrappleSelection::Enabled {
        source,
        binding,
        mechanic,
    } = &equipment.grapple
    {
        match mechanic {
            GrappleMechanicDetail::Q1Threewave { edition } => {
                add(
                    source,
                    [
                        "progs/star.mdl",
                        "sound/weapons/chain1.wav",
                        "sound/blob/land1.wav",
                        "sound/player/axhit2.wav",
                    ]
                    .iter()
                    .map(|name| name.to_string())
                    .collect(),
                );
                if *edition == SourceEdition::Classic {
                    add(
                        source,
                        [
                            "progs/bit.mdl",
                            "sound/weapons/chain2.wav",
                            "sound/weapons/chain3.wav",
                            "sound/weapons/bounce2.wav",
                        ]
                        .iter()
                        .map(|name| name.to_string())
                        .collect(),
                    );
                } else {
                    add(source, vec!["progs/beam.mdl".to_string()]);
                }
                if *binding == GrappleBinding::Slot {
                    add(source, vec!["progs/v_star.mdl".to_string()]);
                }
            }
            GrappleMechanicDetail::Q2Ctf { edition } => {
                let mut paths = vec![
                    "models/weapons/grapple/hook/tris.md2".to_string(),
                    "models/weapons/grapple/hook/skin.pcx".to_string(),
                ];
                for name in ["grfire", "grpull", "grhit", "grhang", "grreset"] {
                    paths.push(format!("sound/weapons/grapple/{name}.wav"));
                }
                add(source, paths);
                if *edition == SourceEdition::Rerelease {
                    add(source, vec!["sound/weapons/grapple/grfly.wav".to_string()]);
                }
                if *binding == GrappleBinding::Slot {
                    add(
                        source,
                        vec![
                            "models/weapons/grapple/tris.md2".to_string(),
                            "models/weapons/grapple/skin.pcx".to_string(),
                        ],
                    );
                }
            }
            GrappleMechanicDetail::Q2Lmctf => {
                let mut paths = vec![
                    "models/objects/ghook/tris.md2".to_string(),
                    "models/objects/ghook/skin.pcx".to_string(),
                ];
                for name in ["grfire", "gflyair", "gpulling", "gkilling", "ghit", "ghitwall"] {
                    paths.push(format!("sound/weapons/grapple/{name}.wav"));
                }
                add(source, paths);
                if *binding == GrappleBinding::Slot {
                    add(
                        source,
                        vec![
                            "models/weapons/v_hook/tris.md2".to_string(),
                            "models/weapons/v_hook/skin.pcx".to_string(),
                        ],
                    );
                }
            }
            GrappleMechanicDetail::Q3Qvm { profile } => {
                let presentation = &profile.presentation;
                let mut paths = vec![
                    profile.module.artifact_path.clone(),
                    presentation.projectile_model.clone(),
                ];
                if *binding == GrappleBinding::Slot {
                    paths.push(presentation.view_anchor.path.clone());
                    paths.push(presentation.view_model.clone());
                    for attachment in &presentation.view_attachments {
                        paths.push(attachment.path.clone());
                    }
                }
                if let QvmGrappleCable::Model { flight, pull, hold, .. } = &presentation.cable {
                    paths.push(flight.clone());
                    paths.push(pull.clone());
                    paths.push(hold.clone());
                }
                for sound in [
                    &presentation.fire_sound,
                    &presentation.attach_sound,
                    &presentation.release_sound,
                    &presentation.pull_sound,
                    &presentation.hang_sound,
                ]
                .into_iter()
                .flatten()
                {
                    paths.push(sound.clone());
                }
                add(source, paths);
            }
        }
    }
    if let HandGrenadeSelection::Enabled { source, edition, .. } = &equipment.hand_grenades {
        let model = if *edition == SourceEdition::Classic {
            "grenade2"
        } else {
            "grenade3"
        };
        let mut paths = vec![
            format!("models/objects/{model}/tris.md2"),
            format!("models/objects/{model}/skin.pcx"),
        ];
        for name in ["hgrena1b", "hgrenc1b", "hgrent1a", "hgrenb1a", "hgrenb2a"] {
            paths.push(format!("sound/weapons/{name}.wav"));
        }
        add(source, paths);
    }
    requests
}

// `addons.ts`.

/// Quaddicted singleplayer Quake catalog URL (`quaddictedCatalogUrl`).
pub const QUADDICTED_CATALOG_URL: &str = "https://www.quaddicted.com/api/v1/?q=%2Btags%3A%22game%3Dquake%22%20%2Btags%3A%22game_mode%3Dsingleplayer%22&fl=sha256,tags,urls,bytes,install";

/// Extraction mapping from an archive member prefix to an install prefix (`AddonPackage` mapping).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddonMapping {
    /// Archive member prefix (`""` matches everything).
    pub from: String,
    /// Install prefix, or `None` to skip the member.
    pub to: Option<String>,
}

/// Installable Quaddicted package (`AddonPackage`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddonPackage {
    /// Package content digest.
    pub digest: ContentDigest,
    /// Package SHA-256 hex.
    pub sha256: String,
    /// Display title.
    pub title: String,
    /// Archive filename.
    pub filename: String,
    /// Release group.
    pub group: String,
    /// Archive byte length.
    pub bytes: u64,
    /// Validated `https://www.quaddicted.com` download URL.
    pub url: String,
    /// Package tags.
    pub tags: Vec<String>,
    /// Start maps.
    pub starts: Vec<String>,
    /// Target game directory.
    pub game_directory: String,
    /// Extraction mappings.
    pub mappings: Vec<AddonMapping>,
    /// Reason the package cannot install, when it cannot.
    pub unavailable: Option<String>,
}

fn addon_strings(value: Option<&SaveJson>) -> Result<Vec<String>, CatalogError> {
    match value {
        None | Some(SaveJson::Null) => Ok(Vec::new()),
        Some(SaveJson::Array(items)) => {
            let mut strings = Vec::new();
            for item in items {
                match item {
                    SaveJson::String(text) => strings.push(text.clone()),
                    _ => return Err(invalid("Invalid Quaddicted string list")),
                }
            }
            Ok(strings)
        }
        Some(_) => Err(invalid("Invalid Quaddicted string list")),
    }
}

fn addon_required_strings(value: Option<&SaveJson>) -> Result<Vec<String>, CatalogError> {
    match value {
        Some(SaveJson::Array(_)) => addon_strings(value),
        _ => Err(invalid("Invalid Quaddicted string list")),
    }
}

/// Values of `name=value` tags (`addonTags`).
#[must_use]
pub fn addon_tags(tags: &[String], name: &str) -> Vec<String> {
    let prefix = format!("{name}=");
    tags.iter()
        .filter(|tag| tag.starts_with(&prefix))
        .map(|tag| tag[prefix.len()..].to_string())
        .collect()
}

/// Normalize a Quaddicted install prefix; installation paths are relative to
/// a virtual Quake base (`pathPrefix`).
fn path_prefix(value: &str) -> Result<String, CatalogError> {
    let path = value
        .strip_prefix("{base}/")
        .or_else(|| value.strip_prefix("{base}"))
        .unwrap_or(value);
    let path = path.strip_prefix('/').unwrap_or(path);
    let path = path.strip_suffix('/').unwrap_or(path);
    if !path.is_empty() {
        contained(path)?;
    }
    if path.is_empty() {
        Ok(String::new())
    } else if value.ends_with('/') {
        Ok(format!("{path}/"))
    } else {
        Ok(path.to_string())
    }
}

/// Validate one Quaddicted download URL: `Ok(None)` skips well-formed
/// non-matching URLs while `Err` rejects malformed ones (donor `new URL`).
fn quaddicted_url(raw: &str, sha256: &str) -> Result<Option<String>, CatalogError> {
    let Some((scheme, rest)) = raw.split_once("://") else {
        return Err(invalid(format!("Invalid Quaddicted package URL: {raw}")));
    };
    if !scheme.eq_ignore_ascii_case("https") {
        return Ok(None);
    }
    let (authority, path) = match rest.find('/') {
        Some(slash) => (&rest[..slash], &rest[slash..]),
        None => (rest, "/"),
    };
    if authority.is_empty() || authority.chars().any(char::is_whitespace) {
        return Err(invalid(format!("Invalid Quaddicted package URL: {raw}")));
    }
    if authority.contains('@') {
        return Ok(None);
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => {
            if port.is_empty() {
                (host, None)
            } else if port.bytes().all(|byte| byte.is_ascii_digit()) && port.len() <= 5 {
                (host, Some(port))
            } else {
                return Err(invalid(format!("Invalid Quaddicted package URL: {raw}")));
            }
        }
        None => (authority, None),
    };
    if let Some(port) = port {
        if port.parse::<u32>().map_or(true, |port| port > 65535) {
            return Err(invalid(format!("Invalid Quaddicted package URL: {raw}")));
        }
    }
    if !host.eq_ignore_ascii_case("www.quaddicted.com") {
        return Ok(None);
    }
    let prefix = format!("/files/by-sha256/{}/{sha256}/", &sha256[..2]);
    if path.starts_with(&prefix) {
        Ok(Some(raw.to_string()))
    } else {
        Ok(None)
    }
}

fn quaddicted_game_directory(commandline: &str) -> String {
    let tokens: Vec<&str> = commandline.split_whitespace().collect();
    for window in tokens.windows(2) {
        if window[0] == "-game"
            && !window[1].is_empty()
            && window[1]
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'+' | b'-'))
        {
            return window[1].to_string();
        }
    }
    "id1".to_string()
}

fn quaddicted_unsupported_option(commandline: &str) -> bool {
    commandline
        .split_whitespace()
        .any(|token| token == "-hipnotic" || token == "-rogue" || token == "-quoth")
}

/// Parse a Quaddicted API catalog (`parseAddonCatalog`).
pub fn parse_addon_catalog(value: &SaveJson) -> Result<Vec<AddonPackage>, CatalogError> {
    let SaveJson::Array(rows) = value else {
        return Err(invalid("Invalid Quaddicted catalog"));
    };
    let mut result = Vec::new();
    for row in rows {
        let SaveJson::Object(_) = row else {
            return Err(invalid("Invalid Quaddicted package"));
        };
        let tags = addon_required_strings(row.get("tags"))?;
        let filename = addon_tags(&tags, "filename").into_iter().next();
        let Some(filename) = filename else {
            continue;
        };
        if !tags.iter().any(|tag| tag == "game=quake")
            || !tags.iter().any(|tag| tag == "game_mode=singleplayer")
            || !filename.ends_with(".zip")
        {
            continue;
        }
        let sha256 = match row.get("sha256") {
            Some(SaveJson::String(sha256)) => sha256.clone(),
            _ => {
                return Err(invalid(format!("Invalid Quaddicted package identity: {filename}")));
            }
        };
        if sha256.len() != 64
            || !sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(invalid(format!("Invalid Quaddicted package identity: {filename}")));
        }
        let bytes = match row.get("bytes") {
            Some(SaveJson::Number(bytes))
                if bytes.trunc() == *bytes && *bytes > 0.0 && *bytes <= MAX_SAFE_INTEGER as f64 =>
            {
                *bytes as u64
            }
            _ => return Err(invalid(format!("Invalid Quaddicted package identity: {filename}"))),
        };
        let mut url = None;
        for raw in addon_strings(row.get("urls"))? {
            if let Some(valid) = quaddicted_url(&raw, &sha256)? {
                url = Some(valid);
                break;
            }
        }
        let Some(url) = url else { continue };
        let mut unavailable = None;
        let commandline = addon_tags(&tags, "commandline").join(" ");
        let game_directory = quaddicted_game_directory(&commandline);
        if quaddicted_unsupported_option(&commandline) {
            unavailable = Some("Requires a source gameplay option not yet supported by this installer".to_string());
        }
        let mut mappings = Vec::new();
        match addon_install_mappings(row.get("install")) {
            Ok(mapped) => mappings = mapped,
            Err(reason) => unavailable = Some(reason),
        }
        let starts = addon_tags(&tags, "startmap")
            .into_iter()
            .filter(|path| {
                !path.is_empty()
                    && path
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'+' | b'.' | b'/' | b'-'))
                    && !path.contains("..")
            })
            .collect();
        result.push(AddonPackage {
            digest: create_content_digest(&sha256)?,
            sha256,
            title: addon_tags(&tags, "title")
                .into_iter()
                .next()
                .unwrap_or_else(|| filename.clone()),
            filename: filename.clone(),
            group: addon_tags(&tags, "release_group")
                .into_iter()
                .next()
                .unwrap_or_else(|| filename.strip_suffix(".zip").unwrap_or(&filename).to_string()),
            bytes,
            url,
            tags,
            starts,
            game_directory,
            mappings,
            unavailable,
        });
    }
    Ok(result)
}

fn addon_install_mappings(install: Option<&SaveJson>) -> Result<Vec<AddonMapping>, String> {
    let fail = |message: &str| -> Result<Vec<AddonMapping>, String> { Err(message.to_string()) };
    let Some(SaveJson::Object(_)) = install else {
        return fail("No authored installation metadata");
    };
    let install = install.expect("checked");
    if let Some(SaveJson::Object(mapping)) = install.get("extractmapping") {
        let mut mappings = Vec::new();
        for (from, to) in mapping {
            match to {
                SaveJson::Null => mappings.push(AddonMapping {
                    from: path_prefix(from).map_err(|error| error.to_string())?,
                    to: None,
                }),
                SaveJson::String(to) => {
                    let to = path_prefix(to).map_err(|error| error.to_string())?;
                    mappings.push(AddonMapping {
                        from: path_prefix(from).map_err(|error| error.to_string())?,
                        to: Some(to),
                    });
                }
                _ => return fail("Unsupported extraction mapping"),
            }
        }
        return Ok(mappings);
    }
    if let Some(SaveJson::String(extract)) = install.get("extract") {
        return Ok(vec![AddonMapping {
            from: String::new(),
            to: Some(path_prefix(extract).map_err(|error| error.to_string())?),
        }]);
    }
    fail("No authored extraction mapping")
}

/// Map an archive member to its install path, or `None` to skip it (`addonInstallPath`).
pub fn addon_install_path(package: &AddonPackage, member: &str) -> Result<Option<String>, CatalogError> {
    contained(member)?;
    let mut mappings = package.mappings.clone();
    mappings.sort_by_key(|mapping| std::cmp::Reverse(mapping.from.len()));
    let mapping = mappings.iter().find(|mapping| {
        if mapping.from.is_empty() || mapping.from.ends_with('/') {
            member.starts_with(&mapping.from)
        } else {
            member == mapping.from
        }
    });
    let Some(mapping) = mapping else {
        return Ok(None);
    };
    let Some(to) = &mapping.to else {
        return Ok(None);
    };
    let path = format!("{to}{}", &member[mapping.from.len()..]);
    contained(&path)?;
    let first = path.split('/').next().unwrap_or("");
    if ["maps", "progs", "gfx", "sound", "music", "env", "textures"].contains(&first) || !path.contains('/') {
        Ok(Some(format!("id1/{path}")))
    } else {
        Ok(Some(path))
    }
}

/// Split text into digit/non-digit runs for numeric version comparison.
fn version_runs(text: &str) -> Vec<String> {
    let mut runs = Vec::new();
    let mut current = String::new();
    let mut current_digit = None;
    for character in text.chars() {
        let digit = character.is_ascii_digit();
        if Some(digit) != current_digit && !current.is_empty() {
            runs.push(std::mem::take(&mut current));
        }
        current_digit = Some(digit);
        current.push(character);
    }
    if !current.is_empty() {
        runs.push(current);
    }
    runs
}

/// Numeric-aware version comparison (donor `localeCompare` with `numeric: true`).
///
/// Digit runs compare by numeric value; other runs compare case-insensitively
/// with an exact tiebreak, approximating English collation for versions.
fn compare_versions(left: &str, right: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let left_runs = version_runs(left);
    let right_runs = version_runs(right);
    for pair in left_runs.iter().zip(right_runs.iter()) {
        let (left, right) = pair;
        if left.bytes().all(|byte| byte.is_ascii_digit()) && right.bytes().all(|byte| byte.is_ascii_digit()) {
            let left = left.trim_start_matches('0');
            let right = right.trim_start_matches('0');
            match left.len().cmp(&right.len()).then_with(|| left.cmp(right)) {
                Ordering::Equal => continue,
                ordering => return ordering,
            }
        } else {
            match left
                .to_lowercase()
                .cmp(&right.to_lowercase())
                .then_with(|| left.cmp(right))
            {
                Ordering::Equal => continue,
                ordering => return ordering,
            }
        }
    }
    left_runs.len().cmp(&right_runs.len())
}

fn dependency_name_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'+' | b'-')
}

/// Parse one quoted `name operator version` requirement at `text[start..]`.
fn parse_dependency_requirement(text: &str, start: usize) -> Option<(usize, String, String, String)> {
    let bytes = text.as_bytes();
    if bytes.get(start) != Some(&b'\'') {
        return None;
    }
    let mut offset = start + 1;
    let name_start = offset;
    while offset < bytes.len() && dependency_name_char(bytes[offset]) {
        offset += 1;
    }
    if offset == name_start {
        return None;
    }
    let name = text[name_start..offset].to_string();
    let operator = if text[offset..].starts_with(">=") || text[offset..].starts_with("<=") {
        let operator = text[offset..offset + 2].to_string();
        offset += 2;
        operator
    } else if text[offset..].starts_with('=') || text[offset..].starts_with('>') || text[offset..].starts_with('<') {
        let operator = text[offset..offset + 1].to_string();
        offset += 1;
        operator
    } else {
        return None;
    };
    let version_start = offset;
    while offset < bytes.len() && dependency_name_char(bytes[offset]) {
        offset += 1;
    }
    if offset == version_start || bytes.get(offset) != Some(&b'\'') {
        return None;
    }
    let version = text[version_start..offset].to_string();
    Some((offset + 1, name, operator, version))
}

/// Find every quoted requirement in an expression (donor `matchAll`).
fn dependency_requirements(expression: &str) -> Vec<(String, String, String)> {
    let mut matches = Vec::new();
    let mut offset = 0;
    while offset < expression.len() {
        if let Some((end, name, operator, version)) = parse_dependency_requirement(expression, offset) {
            matches.push((name, operator, version));
            offset = end;
        } else {
            offset += 1;
        }
    }
    matches
}

/// Split a `provides` tag into its name/version pair.
fn provides_pair(provided: &str) -> Option<(&str, &str)> {
    let stripped = provided.strip_prefix('\'').unwrap_or(provided);
    let stripped = stripped.strip_suffix('\'').unwrap_or(stripped);
    let (name, version) = stripped.split_once('=')?;
    if name.is_empty() || version.is_empty() || name.contains('\'') || name.contains('=') || version.contains('\'') {
        return None;
    }
    Some((name, version))
}

/// Order a package after its dependencies (`resolveAddonPackages`).
pub fn resolve_addon_packages(
    catalog: &[AddonPackage],
    selected: &AddonPackage,
) -> Result<Vec<AddonPackage>, CatalogError> {
    let mut result: Vec<AddonPackage> = Vec::new();
    let mut visiting: HashSet<String> = HashSet::new();
    fn visit(
        catalog: &[AddonPackage],
        item: &AddonPackage,
        result: &mut Vec<AddonPackage>,
        visiting: &mut HashSet<String>,
    ) -> Result<(), CatalogError> {
        if result.iter().any(|existing| existing.sha256 == item.sha256) {
            return Ok(());
        }
        if visiting.contains(&item.sha256) {
            return Err(failed(format!("Cyclic add-on dependency: {}", item.title)));
        }
        if let Some(unavailable) = &item.unavailable {
            return Err(failed(format!("{}: {unavailable}", item.title)));
        }
        visiting.insert(item.sha256.clone());
        let dependencies = addon_tags(&item.tags, "dependency");
        let expressions = addon_tags(&item.tags, "depends");
        if !expressions.is_empty() && dependencies.is_empty() {
            for expression in &expressions {
                let matches = dependency_requirements(expression);
                if matches.is_empty() {
                    return Err(failed(format!("Unsupported dependency requirement: {expression}")));
                }
                for (name, operator, version) in matches {
                    let dependency = catalog.iter().find(|candidate| {
                        addon_tags(&candidate.tags, "provides").iter().any(|provided| {
                            let Some((provided_name, provided_version)) = provides_pair(provided) else {
                                return false;
                            };
                            if provided_name != name {
                                return false;
                            }
                            let compared = compare_versions(provided_version, &version);
                            match operator.as_str() {
                                "=" => compared == std::cmp::Ordering::Equal,
                                ">=" => compared != std::cmp::Ordering::Less,
                                "<=" => compared != std::cmp::Ordering::Greater,
                                ">" => compared == std::cmp::Ordering::Greater,
                                _ => compared == std::cmp::Ordering::Less,
                            }
                        })
                    });
                    let Some(dependency) = dependency else {
                        return Err(failed(format!("Missing dependency: {expression}")));
                    };
                    visit(catalog, dependency, result, visiting)?;
                }
            }
        }
        for name in &dependencies {
            let dependency = catalog.iter().find(|candidate| {
                candidate.filename == format!("{name}.zip") || candidate.filename == *name || candidate.sha256 == *name
            });
            let Some(dependency) = dependency else {
                return Err(failed(format!("Missing dependency: {name}")));
            };
            visit(catalog, dependency, result, visiting)?;
        }
        visiting.remove(&item.sha256);
        result.push(item.clone());
        Ok(())
    }
    visit(catalog, selected, &mut result, &mut visiting)?;
    Ok(result)
}

/// Whether the last path segment is a managed add-on directory.
fn is_managed_addon_segment(segment: &str) -> bool {
    segment.len() == 44
        && segment.starts_with("qd_")
        && segment.as_bytes()[3..23]
            .iter()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        && segment.as_bytes()[23] == b'_'
        && segment.as_bytes()[24..]
            .iter()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

/// Whether a managed add-on directory is hidden (`managedAddonHidden`).
pub fn managed_addon_hidden(root: &Path, directory: &str) -> Result<bool, CatalogError> {
    if !is_managed_addon_segment(directory.rsplit('/').next().unwrap_or("")) {
        return Ok(false);
    }
    contained(directory)?;
    let mut path = root.to_path_buf();
    path.push(".addons");
    path.push("removed");
    for part in directory.split('/') {
        path.push(part);
    }
    match std::fs::metadata(&path) {
        Ok(_) => Ok(true),
        Err(error) if is_not_found(&error) => Ok(false),
        Err(error) => Err(io_error(&path, error)),
    }
}

/// Title recorded by a managed add-on install (`managedAddonTitle`).
pub fn managed_addon_title(root: &Path, content_directory: &str) -> Result<Option<String>, CatalogError> {
    if !is_managed_addon_segment(content_directory.rsplit('/').next().unwrap_or("")) {
        return Ok(None);
    }
    let mut path = root.to_path_buf();
    for part in content_directory.split('/') {
        path.push(part);
    }
    path.push(".quaddicted.json");
    let bytes = std::fs::read(&path).map_err(|error| io_error(&path, error))?;
    let text = String::from_utf8(bytes).map_err(|_| invalid("Invalid managed add-on title"))?;
    let value = parse_save_json(&text)?;
    match value.get("title") {
        Some(SaveJson::String(title)) => Ok(Some(title.clone())),
        _ => Ok(None),
    }
}

// `index.ts` (the two `launch.ts` re-export lines are out of scope and dropped).

/// Inspected archive member (`CatalogArchive` entry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogArchiveEntry {
    /// Member path.
    pub path: String,
    /// Member ordinal.
    pub ordinal: u64,
    /// Member byte length.
    pub byte_length: u64,
}

/// Inspected content archive (`CatalogArchive`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogArchive {
    /// Archive file path.
    pub path: String,
    /// Container format.
    pub format: ArchiveFormat,
    /// File entries in physical order.
    pub entries: Vec<CatalogArchiveEntry>,
}

/// Reachable map (`ContentMap`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentMap {
    /// Resource path.
    pub path: String,
    /// Archive path or loose file path holding the map.
    pub source: String,
    /// Archive member ordinal, or `None` for loose files.
    pub member_index: Option<u64>,
}

/// Product availability (`ProductAvailability`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProductAvailability {
    /// All requirements are present.
    Installed,
    /// Requirements are missing.
    Missing {
        /// Missing requirement paths.
        requirements: Vec<String>,
    },
    /// The product can never resolve.
    Unresolved {
        /// Why the product cannot resolve.
        reason: String,
    },
}

/// Writable user content for a product (`CatalogProduct` user content).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserContent {
    /// User content root for the product.
    pub root: String,
    /// Archives beneath the user root.
    pub archives: Vec<CatalogArchive>,
}

/// Installed catalog product (`CatalogProduct`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogProduct {
    /// Product content identity.
    pub id: ContentId,
    /// Product expectation.
    pub expectation: ProductExpectation,
    /// Product availability.
    pub availability: ProductAvailability,
    /// User archives followed by corpus archives.
    pub archives: Vec<CatalogArchive>,
    /// Loose corpus root, when the content directory exists.
    pub loose_root: Option<String>,
    /// Writable user content, when a user root is configured.
    pub user_content: Option<UserContent>,
    /// User maps followed by corpus maps.
    pub maps: Vec<ContentMap>,
    /// Archive inspection diagnostics.
    pub diagnostics: Vec<String>,
}

/// Remote content base product (`RemoteContentBase`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RemoteContentBase {
    /// QuakeWorld base.
    Q1Quakeworld,
    /// Classic Quake II base.
    Q2ClassicBaseq2,
    /// Rerelease Quake II base.
    Q2RereleaseBaseq2,
    /// Quake III base.
    Q3Baseq3,
}

impl RemoteContentBase {
    /// Stock product identity of the base.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            RemoteContentBase::Q1Quakeworld => "q1-quakeworld",
            RemoteContentBase::Q2ClassicBaseq2 => "q2-classic-baseq2",
            RemoteContentBase::Q2RereleaseBaseq2 => "q2-rerelease-baseq2",
            RemoteContentBase::Q3Baseq3 => "q3-baseq3",
        }
    }
}

/// Remote content selection (`RemoteContentSelection`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteContentSelection {
    /// Remote base product.
    pub base: RemoteContentBase,
    /// Remote game directory.
    pub directory: String,
}

/// Normalize a remote game directory against its base (`remoteContentSelection`).
pub fn remote_content_selection(
    base: RemoteContentBase,
    game_directory: &str,
) -> Result<RemoteContentSelection, CatalogError> {
    let directory = game_directory.to_lowercase();
    if base == RemoteContentBase::Q1Quakeworld && (directory == "qw" || directory == "id1") {
        return Ok(RemoteContentSelection {
            base,
            directory: "qw".to_string(),
        });
    }
    if (base == RemoteContentBase::Q2ClassicBaseq2 || base == RemoteContentBase::Q2RereleaseBaseq2)
        && directory.is_empty()
    {
        return Ok(RemoteContentSelection {
            base,
            directory: "baseq2".to_string(),
        });
    }
    if base == RemoteContentBase::Q3Baseq3 && directory.is_empty() {
        return Ok(RemoteContentSelection {
            base,
            directory: "baseq3".to_string(),
        });
    }
    if directory.is_empty()
        || directory == "."
        || directory.contains("..")
        || !directory
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'+' | b'.' | b'-'))
    {
        return Err(invalid("Remote game directory must be a single safe directory name"));
    }
    Ok(RemoteContentSelection { base, directory })
}

/// Resolve a remote selection to a known or synthetic product (`remoteExpectation`).
fn remote_expectation(
    selection: &RemoteContentSelection,
    products: &[ProductExpectation],
) -> Result<ProductExpectation, CatalogError> {
    let context = remote_content_selection(selection.base, &selection.directory)?;
    let base = products
        .iter()
        .find(|product| product.id == context.base.as_str())
        .ok_or_else(|| failed(format!("Missing remote base product {}", context.base.as_str())))?;
    let directory = format!("{}/{}", posix_dirname(&base.content_directory), context.directory);
    let known = products.iter().find(|product| {
        product.content_directory.to_lowercase() == directory.to_lowercase()
            && product.family == base.family
            && product.edition == base.edition
    });
    if let Some(known) = known {
        let mut visited: HashSet<&str> = HashSet::new();
        let mut current = Some(known);
        while let Some(product) = current {
            if !visited.insert(product.id.as_str()) {
                break;
            }
            if product.id == base.id {
                return Ok(known.clone());
            }
            current = product
                .base_product
                .as_deref()
                .and_then(|base_id| products.iter().find(|product| product.id == base_id));
        }
        return Err(failed(format!(
            "Remote product {} does not inherit {}",
            known.id, base.id
        )));
    }
    Ok(ProductExpectation {
        id: format!("{}-mod-{}", base.id, context.directory),
        family: base.family,
        edition: base.edition.clone(),
        campaign: format!("mod-{}", context.directory),
        title: context.directory.clone(),
        content_directory: directory,
        base_product: Some(base.id.clone()),
        required_content_archives: Vec::new(),
        required_programs: Vec::new(),
        map_witness: None,
        unresolved_reason: None,
    })
}

/// Resolve a remote selection to its product identity (`remoteContentProduct`).
pub fn remote_content_product(
    selection: &RemoteContentSelection,
    products: Option<&[ProductExpectation]>,
) -> Result<String, CatalogError> {
    let stock;
    let products = match products {
        Some(products) => products,
        None => {
            stock = expected_products();
            &stock
        }
    };
    Ok(remote_expectation(selection, products)?.id)
}

/// Installed-content discovery options (`DiscoverContentOptions`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoverContentOptions {
    /// Corpus root holding installed content.
    pub corpus_root: PathBuf,
    /// Writable user content root.
    pub user_content_root: Option<PathBuf>,
    /// Product inventory (stock expectations by default).
    pub products: Option<Vec<ProductExpectation>>,
    /// Catalog generation.
    pub generation: u64,
    /// Whether to discover mods (enabled by default).
    pub discover_mods: bool,
    /// Remote content selection.
    pub remote_content: Option<RemoteContentSelection>,
}

impl DiscoverContentOptions {
    /// Discovery options for a corpus root with default settings.
    #[must_use]
    pub fn new(corpus_root: PathBuf) -> Self {
        Self {
            corpus_root,
            user_content_root: None,
            products: None,
            generation: 0,
            discover_mods: true,
            remote_content: None,
        }
    }
}

/// Mount plan selection (`MountPlanSelection`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountPlanSelection {
    /// Mount plan identity.
    pub id: MountPlanId,
    /// Presentation assets content.
    pub assets: ContentId,
    /// Map geometry content.
    pub geometry: ContentId,
    /// Rules content.
    pub rules: Option<ContentId>,
    /// An explicit presentation choice wins over the rules preset's assets.
    pub explicit_presentation: bool,
    /// Additional content.
    pub additional: Vec<ContentId>,
}

/// Archive format for a filename, or `None` when not an archive (`archiveFormat`).
fn archive_format(name: &str) -> Option<ArchiveFormat> {
    match file_extension(name).as_str() {
        ".pak" => Some(ArchiveFormat::Pak),
        ".pk3" => Some(ArchiveFormat::Pk3),
        ".kpf" => Some(ArchiveFormat::Kpf),
        ".zip" | ".pkz" => Some(ArchiveFormat::Zip),
        _ => None,
    }
}

/// Parse the numeric `pak` prefix (`q2repro` `pakcmp` number parsing).
///
/// The magnitude clamps to `u64::MAX`; in-range values wrap like
/// `BigInt.asUintN(64, value)`.
fn q2_pak_number(tail: &str) -> (u64, &str) {
    let bytes = tail.as_bytes();
    let mut offset = 0;
    while offset < bytes.len() && matches!(bytes[offset], b' ' | b'\t' | b'\n' | b'\r' | b'\x0C' | b'\x0B') {
        offset += 1;
    }
    let negative = if bytes.get(offset) == Some(&b'+') {
        offset += 1;
        false
    } else if bytes.get(offset) == Some(&b'-') {
        offset += 1;
        true
    } else {
        false
    };
    let digits_start = offset;
    while offset < bytes.len() && bytes[offset].is_ascii_digit() {
        offset += 1;
    }
    if offset == digits_start {
        return (0, tail);
    }
    let mut magnitude: u128 = 0;
    for byte in &bytes[digits_start..offset] {
        magnitude = magnitude.saturating_mul(10).saturating_add(u128::from(byte - b'0'));
    }
    let number = if magnitude > u128::from(u64::MAX) {
        u64::MAX
    } else if negative {
        (1u128 << 64).wrapping_sub(magnitude) as u64
    } else {
        magnitude as u64
    };
    (number, &tail[offset..])
}

/// `q2repro` `files.c` `pakcmp`: numeric `pak` prefixes first, then
/// case-insensitive names (`compareQ2Archives`).
fn compare_q2_archives(left: &CatalogArchive, right: &CatalogArchive) -> std::cmp::Ordering {
    let first = Path::new(&left.path)
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let second = Path::new(&right.path)
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let first_pak = first.starts_with("pak");
    let second_pak = second.starts_with("pak");
    if !first_pak || !second_pak {
        return match (first_pak, second_pak) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => first.cmp(&second),
        };
    }
    let (left_number, left_suffix) = q2_pak_number(&first[3..]);
    let (right_number, right_suffix) = q2_pak_number(&second[3..]);
    left_number
        .cmp(&right_number)
        .then_with(|| left_suffix.cmp(right_suffix))
}

/// Basename of an archive path.
fn archive_basename(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Highest priority first, matching each source engine's prepend order
/// (`orderGameArchives`).
#[must_use]
pub fn order_game_archives<'a>(
    product: &ProductExpectation,
    archives: &'a [CatalogArchive],
) -> Vec<&'a CatalogArchive> {
    order_archive_refs(product, archives.iter().collect())
}

/// Borrowed-archive ordering shared by the public entry point and the
/// map/mount winner walks.
fn order_archive_refs<'a>(product: &ProductExpectation, archives: Vec<&'a CatalogArchive>) -> Vec<&'a CatalogArchive> {
    let mut sorted = archives.clone();
    sorted.sort_by_key(|archive| archive_basename(&archive.path));
    if product.family == GameFamily::Q3 {
        let mut pk3: Vec<&CatalogArchive> = sorted
            .into_iter()
            .filter(|archive| archive.format == ArchiveFormat::Pk3)
            .collect();
        pk3.sort_by(|left, right| {
            archive_basename(&left.path)
                .to_lowercase()
                .cmp(&archive_basename(&right.path).to_lowercase())
        });
        pk3.reverse();
        return pk3;
    }
    if product.family == GameFamily::Q2 {
        let mut paks: Vec<&CatalogArchive> = archives
            .into_iter()
            .filter(|archive| {
                archive.format == ArchiveFormat::Pak
                    || archive.format == ArchiveFormat::Zip && archive.path.to_lowercase().ends_with(".pkz")
            })
            .collect();
        paks.sort_by(|left, right| compare_q2_archives(left, right));
        paks.reverse();
        return paks;
    }
    let mut numbered = Vec::new();
    for index in 0..sorted.len() + 1 {
        let wanted = format!("pak{index}.pak");
        match sorted
            .iter()
            .find(|archive| archive_basename(&archive.path).to_lowercase() == wanted)
        {
            Some(archive) => numbered.push(*archive),
            None => break,
        }
    }
    let mut ordered = numbered;
    for archive in sorted {
        if archive.format == ArchiveFormat::Pk3 || archive.format == ArchiveFormat::Kpf {
            ordered.push(archive);
        }
    }
    ordered.reverse();
    ordered
}

/// One product directory: loose root plus its archives (`productDirectories` entry).
struct ProductDirectory<'a> {
    root: Option<&'a str>,
    archives: Vec<&'a CatalogArchive>,
}

/// User directory first, then the corpus directory without user archives
/// (`productDirectories`).
fn product_directories(product: &CatalogProduct) -> Vec<ProductDirectory<'_>> {
    match &product.user_content {
        None => vec![ProductDirectory {
            root: product.loose_root.as_deref(),
            archives: product.archives.iter().collect(),
        }],
        Some(user) => {
            let user_paths: HashSet<&str> = user.archives.iter().map(|archive| archive.path.as_str()).collect();
            vec![
                ProductDirectory {
                    root: Some(user.root.as_str()),
                    archives: user.archives.iter().collect(),
                },
                ProductDirectory {
                    root: product.loose_root.as_deref(),
                    archives: product
                        .archives
                        .iter()
                        .filter(|archive| !user_paths.contains(archive.path.as_str()))
                        .collect(),
                },
            ]
        }
    }
}

/// Read a loose Q3 mod description, home before base, bounded to 48 bytes
/// (`modDescription`; Q3 `FS_GetModList`).
fn mod_description(product: &ProductExpectation, roots: &[PathBuf]) -> Result<ProductExpectation, CatalogError> {
    if product.family != GameFamily::Q3 {
        return Ok(product.clone());
    }
    for root in roots {
        let path = find_content_path(
            root,
            &format!("{}/description.txt", product.content_directory),
            PathComparison::CaseInsensitive,
        )?;
        let Some(path) = path else { continue };
        let mut file = File::open(&path).map_err(|error| io_error(&path, error))?;
        let mut bytes = [0u8; 48];
        let mut read = 0;
        while read < bytes.len() {
            match file.read(&mut bytes[read..]) {
                Ok(0) => break,
                Ok(count) => read += count,
                Err(error) => return Err(io_error(&path, error)),
            }
        }
        if read == 0 {
            return Ok(product.clone());
        }
        let end = bytes[..read].iter().position(|byte| *byte == 0).unwrap_or(read);
        let title: String = bytes[..end].iter().map(|byte| *byte as char).collect();
        return Ok(ProductExpectation {
            title,
            ..product.clone()
        });
    }
    Ok(product.clone())
}

/// Whether a lowercased filename is a Quake program file
/// (`/^(?:qw?progs\.dat|progs\.dat|game.*\.(?:dll|so))$/i`).
fn is_program_file(lower: &str) -> bool {
    fn is_progs_core(text: &str) -> bool {
        text.len() == 9 && text.as_bytes()[..5] == *b"progs" && text.as_bytes()[6..] == *b"dat"
    }
    if let Some(after_q) = lower.strip_prefix('q') {
        let core = after_q.strip_prefix('w').unwrap_or(after_q);
        if is_progs_core(core) {
            return true;
        }
    }
    if is_progs_core(lower) {
        return true;
    }
    if let Some(rest) = lower.strip_prefix("game") {
        if rest.len() >= 4 && rest.ends_with("dll") {
            return true;
        }
        if rest.len() >= 3 && rest.ends_with("so") {
            return true;
        }
    }
    false
}

/// Whether a lowercased program name is a Quake II game module
/// (`/^game[^/]*\.(?:dll|so)$/`).
fn is_q2_game_module(lower: &str) -> bool {
    lower.starts_with("game") && !lower.contains('/') && (lower.ends_with(".dll") || lower.ends_with(".so"))
}

/// Whether a directory entry name is a valid mod identity.
fn is_valid_mod_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    match bytes.next() {
        Some(byte) if byte.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'+' | b'-'))
}

/// Read directory entries in OS order, mapping I/O failures.
fn read_dir_entries(path: &Path) -> Result<Vec<std::fs::DirEntry>, CatalogError> {
    let entries = std::fs::read_dir(path).map_err(|error| io_error(path, error))?;
    let mut listed = Vec::new();
    for entry in entries {
        listed.push(entry.map_err(|error| io_error(path, error))?);
    }
    Ok(listed)
}

/// Discover mod products beneath a content root (`discoverMods`).
fn discover_mods(
    root: &Path,
    products: &[ProductExpectation],
    inspect: &mut dyn FnMut(&Path, ArchiveFormat) -> Result<CatalogArchive, CatalogError>,
) -> Result<Vec<ProductExpectation>, CatalogError> {
    let mut roots: Vec<(&str, &ProductExpectation)> = Vec::new();
    for product in products {
        if product.base_product.is_none()
            && !roots
                .iter()
                .any(|(directory, _)| *directory == posix_dirname(&product.content_directory))
        {
            roots.push((posix_dirname(&product.content_directory), product));
        }
    }
    let mut found = Vec::new();
    for (directory, base) in roots {
        let installed = find_content_path(root, directory, PathComparison::CaseInsensitive)?;
        let Some(installed) = installed else { continue };
        for entry in read_dir_entries(&installed)? {
            let name = entry.file_name().to_string_lossy().into_owned();
            let content_directory = format!("{directory}/{name}");
            let file_type = entry.file_type().map_err(|error| io_error(&entry.path(), error))?;
            if !file_type.is_dir() || name.to_lowercase() == "rerelease" {
                continue;
            }
            let known: Vec<&ProductExpectation> = products
                .iter()
                .filter(|product| product.content_directory.to_lowercase() == content_directory.to_lowercase())
                .collect();
            if !known.is_empty()
                && (base.family != GameFamily::Q1 || known.iter().any(|product| product.edition == "quakeworld"))
            {
                continue;
            }
            if managed_addon_hidden(root, &content_directory)? {
                continue;
            }
            let mod_dir = installed.join(&name);
            let members = read_dir_entries(&mod_dir)?;
            let member_names: Vec<String> = members
                .iter()
                .map(|member| member.file_name().to_string_lossy().into_owned())
                .collect();
            let member_types: Vec<std::fs::FileType> = members
                .iter()
                .map(|member| member.file_type().map_err(|error| io_error(&member.path(), error)))
                .collect::<Result<_, _>>()?;
            let archives: Vec<&str> = member_names
                .iter()
                .zip(member_types.iter())
                .filter(|(name, file_type)| file_type.is_file() && archive_format(name).is_some())
                .map(|(name, _)| name.as_str())
                .collect();
            let has_content = !archives.is_empty()
                || member_names.iter().zip(member_types.iter()).any(|(name, file_type)| {
                    file_type.is_dir() && ["maps", "models", "vm"].contains(&name.to_lowercase().as_str())
                        || file_type.is_file() && is_program_file(&name.to_lowercase())
                });
            if !has_content {
                continue;
            }
            if !is_valid_mod_name(&name) {
                return Err(invalid(format!(
                    "Mod directory needs a valid content identity: {content_directory}"
                )));
            }
            let mut program_names: Vec<String> = member_names
                .iter()
                .zip(member_types.iter())
                .filter(|(_, file_type)| file_type.is_file())
                .map(|(name, _)| name.to_lowercase())
                .collect();
            if base.family == GameFamily::Q3
                && find_content_path(&mod_dir, "vm/qagame.qvm", PathComparison::CaseInsensitive)?.is_some()
            {
                program_names.push("vm/qagame.qvm".to_string());
            }
            for archive in &archives {
                let Some(format) = archive_format(archive) else {
                    continue;
                };
                if base.family == GameFamily::Q2
                    && format != ArchiveFormat::Pak
                    && !(format == ArchiveFormat::Zip && archive.to_lowercase().ends_with(".pkz"))
                {
                    continue;
                }
                if let Ok(inspected) = inspect(&mod_dir.join(archive), format) {
                    program_names.extend(inspected.entries.iter().map(|entry| entry.path.to_lowercase()));
                }
            }
            let mut variants = vec![base];
            let quakeworld = if base.family == GameFamily::Q1 {
                products.iter().find(|product| {
                    product.edition == "quakeworld" && posix_dirname(&product.content_directory) == directory
                })
            } else {
                None
            };
            if let Some(quakeworld) = quakeworld {
                if program_names.iter().any(|name| name == "qwprogs.dat") {
                    variants.push(quakeworld);
                }
            }
            let q2_library = if base.edition == "rerelease" {
                "game_x64.dll"
            } else {
                "gamex86.dll"
            };
            let q2_windows_game = base.family == GameFamily::Q2 && program_names.iter().any(|name| name == q2_library);
            let q2_other_game =
                base.family == GameFamily::Q2 && program_names.iter().any(|name| is_q2_game_module(name));
            for variant in variants {
                if known.iter().any(|product| product.edition == variant.edition) {
                    continue;
                }
                let title = managed_addon_title(root, &content_directory)?.unwrap_or_else(|| {
                    if variant.edition == "quakeworld" {
                        format!("{name} (QuakeWorld)")
                    } else {
                        name.clone()
                    }
                });
                let required_programs = if variant.edition == "quakeworld" {
                    vec!["qwprogs.dat".to_string()]
                } else if q2_windows_game {
                    vec![q2_library.to_string()]
                } else if variant.family == GameFamily::Q1 && program_names.iter().any(|name| name == "progs.dat") {
                    vec!["progs.dat".to_string()]
                } else if variant.family == GameFamily::Q3 && program_names.iter().any(|name| name == "vm/qagame.qvm") {
                    vec!["vm/qagame.qvm".to_string()]
                } else {
                    Vec::new()
                };
                found.push(ProductExpectation {
                    id: format!("{}-{}-{name}", family_name(variant.family), variant.edition),
                    family: variant.family,
                    edition: variant.edition.clone(),
                    campaign: name.clone(),
                    title,
                    content_directory: content_directory.clone(),
                    base_product: Some(variant.id.clone()),
                    required_content_archives: archives
                        .iter()
                        .map(|archive| format!("{content_directory}/{archive}"))
                        .collect(),
                    required_programs,
                    map_witness: None,
                    unresolved_reason: if q2_other_game && !q2_windows_game {
                        Some(format!(
                            "This Quake II add-on needs {}; its installed game module is unsupported.",
                            if base.edition == "rerelease" {
                                "a Windows x64 game_x64.dll"
                            } else {
                                "a Windows i386 gamex86.dll"
                            }
                        ))
                    } else {
                        None
                    },
                });
            }
        }
    }
    Ok(found)
}

/// Loose `.bsp` files beneath a product root (`looseMaps`).
fn loose_maps(root: &Path) -> Result<Vec<ContentMap>, CatalogError> {
    let maps_root = find_content_path(root, "maps", PathComparison::CaseInsensitive)?;
    let Some(maps_root) = maps_root else {
        return Ok(Vec::new());
    };
    fn walk(directory: &Path, root: &Path, result: &mut Vec<ContentMap>) -> Result<(), CatalogError> {
        for entry in read_dir_entries(directory)? {
            let path = directory.join(entry.file_name());
            let file_type = entry.file_type().map_err(|error| io_error(&path, error))?;
            if file_type.is_dir() {
                walk(&path, root, result)?;
            } else if file_type.is_file() && entry.file_name().to_string_lossy().to_lowercase().ends_with(".bsp") {
                let relative = path
                    .strip_prefix(root)
                    .map_err(|_| invalid(format!("Map escapes content root: {}", path.to_string_lossy())))?;
                let mut parts = Vec::new();
                for part in relative.components() {
                    parts.push(part.as_os_str().to_string_lossy().into_owned());
                }
                result.push(ContentMap {
                    path: parts.join("/"),
                    source: path.to_string_lossy().into_owned(),
                    member_index: None,
                });
            }
        }
        Ok(())
    }
    let mut result = Vec::new();
    walk(&maps_root, root, &mut result)?;
    result.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(result)
}

/// Physical path key of a mount.
fn mount_key(mount: &ContentMount) -> &str {
    match mount {
        ContentMount::Archive(mount) => mount.archive_path.as_str(),
        ContentMount::Loose(mount) => mount.root_path.as_str(),
    }
}

/// Installed content catalog (`InstalledCatalog`).
#[derive(Debug, Clone)]
pub struct InstalledCatalog {
    /// Resolved corpus root.
    pub corpus_root: String,
    /// Installed products.
    pub products: Vec<CatalogProduct>,
    /// Root rerelease archives.
    pub root_archives: Vec<CatalogArchive>,
    /// Catalog generation.
    pub generation: u64,
    /// Resolved user content root.
    pub user_content_root: Option<String>,
    by_id: HashMap<String, usize>,
}

impl InstalledCatalog {
    /// Build a catalog, rejecting duplicate product identities.
    pub fn new(
        corpus_root: String,
        products: Vec<CatalogProduct>,
        root_archives: Vec<CatalogArchive>,
        generation: u64,
        user_content_root: Option<String>,
    ) -> Result<Self, CatalogError> {
        let mut by_id = HashMap::new();
        for (index, product) in products.iter().enumerate() {
            if by_id.contains_key(product.id.as_str()) || by_id.contains_key(product.expectation.id.as_str()) {
                return Err(invalid(format!("Duplicate catalog identity: {}", product.id.as_str())));
            }
            by_id.insert(product.id.as_str().to_string(), index);
            by_id.insert(product.expectation.id.clone(), index);
        }
        Ok(Self {
            corpus_root,
            products,
            root_archives,
            generation,
            user_content_root,
            by_id,
        })
    }

    /// Look up a product by content or expectation identity (`product`).
    pub fn product(&self, id: &str) -> Result<&CatalogProduct, CatalogError> {
        self.by_id
            .get(id)
            .and_then(|index| self.products.get(*index))
            .ok_or_else(|| invalid(format!("Unknown requested content or mod: {id}")))
    }

    /// Look up an installed product, reporting missing requirements (`require`).
    pub fn require(&self, id: &str) -> Result<&CatalogProduct, CatalogError> {
        let product = self.product(id)?;
        match &product.availability {
            ProductAvailability::Installed => Ok(product),
            ProductAvailability::Missing { requirements } => {
                Err(failed(format!("Content {id} requires: {}", requirements.join(", "))))
            }
            ProductAvailability::Unresolved { reason } => Err(failed(format!("Content {id} is unresolved: {reason}"))),
        }
    }

    /// Reachable map winners, including the campaign's base content (`mapsFor`).
    pub fn maps_for(&self, id: &str) -> Result<Vec<ContentMap>, CatalogError> {
        fn visit(
            catalog: &InstalledCatalog,
            product: &CatalogProduct,
            winners: &mut Vec<ContentMap>,
            seen: &mut HashSet<String>,
            visited: &mut HashSet<String>,
        ) -> Result<(), CatalogError> {
            if !visited.insert(product.id.as_str().to_string()) {
                return Err(failed(format!(
                    "Cyclic base content dependency: {}",
                    product.id.as_str()
                )));
            }
            let mut add = |map: &ContentMap| {
                if seen.insert(map.path.to_lowercase()) {
                    winners.push(map.clone());
                }
            };
            for directory in product_directories(product) {
                for archive in order_archive_refs(&product.expectation, directory.archives.clone()) {
                    let maps: Vec<&ContentMap> = product.maps.iter().filter(|map| map.source == archive.path).collect();
                    if archive.format == ArchiveFormat::Pak {
                        for map in maps {
                            add(map);
                        }
                    } else {
                        for map in maps.into_iter().rev() {
                            add(map);
                        }
                    }
                }
                if let Some(root) = directory.root {
                    for map in &product.maps {
                        if map.member_index.is_none() && lexical_join(root, &map.path) == map.source {
                            add(map);
                        }
                    }
                }
            }
            if let Some(base) = &product.expectation.base_product {
                let base_id = base.clone();
                visit(catalog, catalog.product(&base_id)?, winners, seen, visited)?;
            }
            Ok(())
        }
        let mut winners = Vec::new();
        let mut seen = HashSet::new();
        let mut visited = HashSet::new();
        visit(self, self.product(id)?, &mut winners, &mut seen, &mut visited)?;
        Ok(winners)
    }

    /// Add one archive mount unless its path is already mounted.
    ///
    /// Discovery never hashes: the archive digest stays lazy until a
    /// real consumer (download verification, save provenance, pure
    /// checks) forces it.
    fn add_archive(
        &self,
        product: &CatalogProduct,
        archive: &CatalogArchive,
        mounts: &mut Vec<ContentMount>,
        paths: &mut HashSet<String>,
    ) -> Result<(), CatalogError> {
        if !paths.insert(archive.path.clone()) {
            return Ok(());
        }
        let name = hex_lower(relative_posix(&self.corpus_root, &archive.path).as_bytes());
        mounts.push(ContentMount::Archive(ArchiveMount {
            identity: create_mount_identity(
                create_mount_id(&product.expectation.id, &name)?,
                product.id.clone(),
                self.generation,
            )?,
            format: archive.format,
            archive_path: archive.path.clone(),
            archive_digest: LazyArchiveDigest::uncomputed(),
        }));
        Ok(())
    }

    /// Mounts for content plus its base chain (`mountsFor`).
    pub fn mounts_for(&self, id: &str) -> Result<Vec<ContentMount>, CatalogError> {
        fn visit(
            catalog: &InstalledCatalog,
            content: &str,
            mounts: &mut Vec<ContentMount>,
            paths: &mut HashSet<String>,
            visited: &mut HashSet<String>,
        ) -> Result<(), CatalogError> {
            let product = catalog.require(content)?;
            if !visited.insert(product.id.as_str().to_string()) {
                return Err(failed(format!("Cyclic base content dependency: {content}")));
            }
            let expectation = product.expectation.clone();
            let product_id = product.id.clone();
            let loose_root = product.loose_root.clone();
            let directories = product_directories(product);
            for directory in &directories {
                for archive in order_archive_refs(&expectation, directory.archives.clone()) {
                    catalog.add_archive(catalog.product(product_id.as_str())?, archive, mounts, paths)?;
                }
                if let Some(root) = directory.root {
                    if paths.insert(root.to_string()) {
                        let name = hex_lower(relative_posix(&catalog.corpus_root, root).as_bytes());
                        mounts.push(ContentMount::Loose(LooseMount {
                            identity: create_mount_identity(
                                create_mount_id(&expectation.id, &format!("loose-{name}"))?,
                                product_id.clone(),
                                catalog.generation,
                            )?,
                            root_path: root.to_string(),
                        }));
                    }
                }
            }
            if let Some(base) = &expectation.base_product {
                visit(catalog, base, mounts, paths, visited)?;
            }
            if expectation.edition == "rerelease" && expectation.base_product.is_none() {
                let anchor = loose_root.unwrap_or_else(|| {
                    Path::new(&catalog.corpus_root)
                        .join(&expectation.content_directory)
                        .to_string_lossy()
                        .into_owned()
                });
                let root = Path::new(&anchor)
                    .parent()
                    .map(|parent| parent.to_string_lossy().into_owned());
                for archive in catalog
                    .root_archives
                    .iter()
                    .filter(|candidate| {
                        Path::new(&candidate.path)
                            .parent()
                            .map(|parent| parent.to_string_lossy().into_owned())
                            == root
                    })
                    .cloned()
                    .collect::<Vec<_>>()
                {
                    catalog.add_archive(catalog.product(product_id.as_str())?, &archive, mounts, paths)?;
                }
            }
            Ok(())
        }
        let mut mounts = Vec::new();
        let mut paths = HashSet::new();
        let mut visited = HashSet::new();
        visit(self, id, &mut mounts, &mut paths, &mut visited)?;
        Ok(mounts)
    }

    /// Resolve a mount plan over asset, geometry, and extra content (`createMountPlan`).
    pub fn create_mount_plan(&self, selection: &MountPlanSelection) -> Result<ResolvedMountPlan, CatalogError> {
        let geometry = self.require(selection.geometry.as_str())?.id.as_str().to_string();
        let mut assets = self.require(selection.assets.as_str())?.id.as_str().to_string();
        let mut required_rules_assets: Option<String> = None;
        if let Some(rules) = &selection.rules {
            let rules_product = self.require(rules.as_str())?;
            if rules_product.expectation.family == GameFamily::Q2 && rules_product.expectation.edition == "rerelease" {
                required_rules_assets = Some(rules_product.id.as_str().to_string());
                if !selection.explicit_presentation {
                    assets = rules_product.id.as_str().to_string();
                }
            }
        }
        let asset_mounts = self.mounts_for(&assets)?;
        let geometry_mounts = self.mounts_for(&geometry)?;
        let mut additional = Vec::new();
        if let Some(required) = &required_rules_assets {
            additional.extend(self.mounts_for(required)?);
        }
        for id in &selection.additional {
            additional.extend(self.mounts_for(id.as_str())?);
        }
        let mut mounts: Vec<ContentMount> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for mount in asset_mounts
            .iter()
            .chain(geometry_mounts.iter())
            .chain(additional.iter())
        {
            if seen.insert(mount_key(mount).to_string()) {
                mounts.push(mount.clone());
            }
        }
        let order = |first: &[ContentMount]| -> Vec<MountId> {
            let mut ids: Vec<MountId> = Vec::new();
            let mut seen_ids: HashSet<MountId> = HashSet::new();
            for mount in first.iter().chain(mounts.iter()) {
                if let Some(canonical) = mounts.iter().find(|candidate| mount_key(candidate) == mount_key(mount)) {
                    if seen_ids.insert(canonical.identity().id.clone()) {
                        ids.push(canonical.identity().id.clone());
                    }
                }
            }
            ids
        };
        let default_order = order(&asset_mounts);
        let prefix_orders = if assets == geometry {
            Vec::new()
        } else {
            vec![PrefixMountOrder {
                prefix: "maps/".to_string(),
                mounts: order(&geometry_mounts),
            }]
        };
        Ok(ResolvedMountPlan {
            id: selection.id.clone(),
            mounts,
            default_order,
            prefix_orders,
        })
    }

    /// Read a resource through the content's mounts (`read`).
    pub fn read(&self, content: &str, path: &str) -> Result<Vec<u8>, CatalogError> {
        let mounts = self.mounts_for(content)?;
        let plan = ResolvedMountPlan {
            id: create_mount_plan_id("catalog", &self.generation.to_string())?,
            mounts: mounts.clone(),
            default_order: mounts.iter().map(|mount| mount.identity().id.clone()).collect(),
            prefix_orders: Vec::new(),
        };
        let opened = open_mount_plan(&plan, OpenMountOptions::default())?;
        Ok(opened.read(ResourceRef::Path(path))?)
    }

    /// Resolve authored starts for a rerelease Quake II product (`authoredStartsFor`).
    pub fn authored_starts_for(&self, id: &str) -> Result<Option<AuthoredStartCatalog>, CatalogError> {
        self.authored_starts_for_with(id, open_mount_plan)
    }

    /// Resolve authored starts with an injectable mount opener (`authoredStartsFor`).
    pub fn authored_starts_for_with(
        &self,
        id: &str,
        open: impl FnOnce(&ResolvedMountPlan, OpenMountOptions) -> Result<MountedContent, MountError>,
    ) -> Result<Option<AuthoredStartCatalog>, CatalogError> {
        let product = self.require(id)?;
        if product.expectation.family != GameFamily::Q2 || product.expectation.edition != "rerelease" {
            return Ok(None);
        }
        let campaign = product.expectation.campaign.clone();
        let mounts = self.mounts_for(product.id.as_str())?;
        let plan = ResolvedMountPlan {
            id: create_mount_plan_id("catalog", &self.generation.to_string())?,
            mounts: mounts.clone(),
            default_order: mounts.iter().map(|mount| mount.identity().id.clone()).collect(),
            prefix_orders: Vec::new(),
        };
        let opened = open(&plan, OpenMountOptions::default())?;
        let Some(resource) = opened.open("mapdb.json", |_| true)? else {
            return Ok(None);
        };
        let starts = parse_authored_starts(&resource.bytes, &campaign)?;
        Ok(Some(AuthoredStartCatalog {
            resource: resource.reference,
            episode: starts.episode,
            starts: starts.starts,
        }))
    }
}

/// Inspect and memoize one archive (`inspect` in `discoverInstalledContent`).
fn inspect_archive(
    cache: &mut HashMap<String, CatalogArchive>,
    path: &Path,
    format: ArchiveFormat,
) -> Result<CatalogArchive, CatalogError> {
    let key = path.to_string_lossy().into_owned();
    if let Some(archive) = cache.get(&key) {
        return Ok(archive.clone());
    }
    let archive = open_archive(path, Some(format))?;
    let inspected = CatalogArchive {
        path: key.clone(),
        format,
        entries: archive
            .entries
            .iter()
            .filter(|entry| !entry.is_directory())
            .map(|entry| CatalogArchiveEntry {
                path: entry.path().to_string(),
                ordinal: entry.ordinal() as u64,
                byte_length: entry.byte_length(),
            })
            .collect(),
    };
    archive.close();
    cache.insert(key, inspected.clone());
    Ok(inspected)
}

/// Discover installed content beneath a corpus root (`discoverInstalledContent`).
pub fn discover_installed_content(options: &DiscoverContentOptions) -> Result<InstalledCatalog, CatalogError> {
    let corpus_root = resolve_path(&options.corpus_root);
    let user_content_root = options.user_content_root.as_deref().map(resolve_path);
    let generation = options.generation;
    if generation > MAX_SAFE_INTEGER {
        return Err(invalid("Catalog generation must be a nonnegative integer"));
    }
    let stock = options.products.clone().unwrap_or_else(expected_products);
    let remote_product = options
        .remote_content
        .as_ref()
        .map(|selected| remote_expectation(selected, &stock))
        .transpose()?;
    let mut remote_overlays: HashSet<String> = HashSet::new();
    if let (Some(remote), Some(selected)) = (&remote_product, &options.remote_content) {
        if remote.id != selected.base.as_str() {
            remote_overlays.insert(remote.id.clone());
        }
    }
    if options.remote_content.as_ref().map(|selected| selected.base) == Some(RemoteContentBase::Q1Quakeworld) {
        remote_overlays.insert(RemoteContentBase::Q1Quakeworld.as_str().to_string());
    }
    let mut expected = stock.clone();
    if let Some(remote) = &remote_product {
        if !expected.iter().any(|product| product.id == remote.id) {
            expected.push(remote.clone());
        }
    }
    let mut archives: HashMap<String, CatalogArchive> = HashMap::new();
    let mut inspect = |path: &Path, format: ArchiveFormat| inspect_archive(&mut archives, path, format);
    let corpus_mods = if !options.discover_mods {
        Vec::new()
    } else {
        discover_mods(&corpus_root, &expected, &mut inspect)?
    };
    let user_mods = if !options.discover_mods || user_content_root.is_none() {
        Vec::new()
    } else {
        let mut combined = expected.clone();
        combined.extend(corpus_mods.iter().cloned());
        discover_mods(
            user_content_root.as_deref().unwrap_or(&corpus_root),
            &combined,
            &mut inspect,
        )?
    };
    let mut user_mod_ids: HashSet<String> = user_mods.iter().map(|product| product.id.clone()).collect();
    user_mod_ids.extend(remote_overlays.iter().cloned());
    let mut description_roots = Vec::new();
    if let Some(user_root) = &user_content_root {
        description_roots.push(user_root.clone());
    }
    description_roots.push(corpus_root.clone());
    let mut candidates = expected.clone();
    candidates.extend(corpus_mods.iter().cloned());
    candidates.extend(user_mods.iter().cloned());
    let mut expectations = Vec::new();
    for product in &candidates {
        if stock.iter().any(|known| known.id == product.id) {
            expectations.push(product.clone());
        } else {
            expectations.push(mod_description(product, &description_roots)?);
        }
    }
    let mut products = Vec::new();
    let mut root_archives: Vec<CatalogArchive> = Vec::new();
    let mut root_archive_paths: HashSet<String> = HashSet::new();
    for expectation in &expectations {
        normalize_resource_path(&expectation.content_directory)?;
        let id = create_content_id(&ContentIdentity {
            family: expectation.family,
            edition: expectation.edition.clone(),
            package: expectation.campaign.clone(),
            revision: "installed".to_string(),
        })?;
        let root_path = find_content_path(
            &corpus_root,
            &expectation.content_directory,
            PathComparison::CaseInsensitive,
        )?;
        let loose_root = match &root_path {
            Some(path) => {
                let is_dir = std::fs::metadata(path).map_err(|error| io_error(path, error))?.is_dir();
                if is_dir {
                    Some(path.to_string_lossy().into_owned())
                } else {
                    None
                }
            }
            None => None,
        };
        let mut found = Vec::new();
        let mut diagnostics = Vec::new();
        if let Some(loose_root) = &loose_root {
            for entry in read_dir_entries(Path::new(loose_root))? {
                let name = entry.file_name().to_string_lossy().into_owned();
                let Some(format) = archive_format(&name) else { continue };
                let file_type = entry.file_type().map_err(|error| io_error(&entry.path(), error))?;
                if !file_type.is_file() {
                    continue;
                }
                match inspect(&Path::new(loose_root).join(&name), format) {
                    Ok(archive) => found.push(archive),
                    Err(error) => diagnostics.push(format!("{name}: {error}")),
                }
            }
            if expectation.edition == "rerelease" {
                let kpf = if expectation.family == GameFamily::Q1 {
                    "QuakeEX.kpf"
                } else {
                    "Q2Game.kpf"
                };
                let parent = Path::new(loose_root)
                    .parent()
                    .unwrap_or(Path::new(loose_root))
                    .to_path_buf();
                if let Some(path) = find_content_path(&parent, kpf, PathComparison::CaseInsensitive)? {
                    let key = path.to_string_lossy().into_owned();
                    if !root_archive_paths.contains(&key) {
                        match inspect(&path, ArchiveFormat::Kpf) {
                            Ok(archive) => {
                                root_archive_paths.insert(key);
                                root_archives.push(archive);
                            }
                            Err(error) => diagnostics.push(format!("{kpf}: {error}")),
                        }
                    }
                }
            }
        }
        let user_root = if user_content_root.is_none() || user_content_root.as_deref() == Some(corpus_root.as_path()) {
            None
        } else {
            let user_root = user_content_root.clone().unwrap_or_else(|| corpus_root.clone());
            match find_content_path(
                &user_root,
                &expectation.content_directory,
                PathComparison::CaseInsensitive,
            )? {
                Some(path) => Some(path),
                None => Some(user_product_directory(&user_root, &expectation.content_directory)?),
            }
        };
        let mut user_archives = Vec::new();
        let mut user_diagnostics = Vec::new();
        if let Some(user_root) = &user_root {
            let entries = match std::fs::read_dir(user_root) {
                Ok(entries) => entries
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| io_error(user_root, error))?,
                Err(error) if is_not_found(&error) => Vec::new(),
                Err(error) => return Err(io_error(user_root, error)),
            };
            for entry in entries {
                let name = entry.file_name().to_string_lossy().into_owned();
                let Some(format) = archive_format(&name) else { continue };
                let file_type = entry.file_type().map_err(|error| io_error(&entry.path(), error))?;
                if !file_type.is_file() {
                    continue;
                }
                match inspect(&user_root.join(&name), format) {
                    Ok(archive) => user_archives.push(archive),
                    Err(error) => user_diagnostics.push(format!("{name}: {error}")),
                }
            }
        }
        let archive_maps = |archives: &[CatalogArchive]| -> Vec<ContentMap> {
            archives
                .iter()
                .flat_map(|archive| {
                    archive
                        .entries
                        .iter()
                        .filter(|entry| {
                            entry.path.len() >= 10
                                && entry.path.to_lowercase().starts_with("maps/")
                                && entry.path.to_lowercase().ends_with(".bsp")
                        })
                        .map(|entry| ContentMap {
                            path: entry.path.clone(),
                            source: archive.path.clone(),
                            member_index: Some(entry.ordinal),
                        })
                })
                .collect()
        };
        let mut corpus_maps = archive_maps(&found);
        if let Some(loose_root) = &loose_root {
            corpus_maps.extend(loose_maps(Path::new(loose_root))?);
        }
        let mut all_maps = archive_maps(&user_archives);
        if let Some(user_root) = &user_root {
            all_maps.extend(loose_maps(user_root)?);
        }
        all_maps.extend(corpus_maps.iter().cloned());
        let mut requirements = diagnostics.clone();
        if user_mod_ids.contains(&expectation.id) {
            requirements.extend(user_diagnostics.iter().cloned());
        }
        if !remote_overlays.contains(&expectation.id) {
            for archive in &expectation.required_content_archives {
                let mut path = find_content_path(&corpus_root, archive, PathComparison::CaseInsensitive)?;
                if path.is_none() && user_mod_ids.contains(&expectation.id) {
                    if let Some(user_root) = &user_content_root {
                        path = find_content_path(user_root, archive, PathComparison::CaseInsensitive)?;
                    }
                }
                let present = match &path {
                    Some(path) => std::fs::metadata(path)
                        .map_err(|error| io_error(path, error))?
                        .is_file(),
                    None => false,
                };
                if !present {
                    requirements.push(archive.clone());
                }
            }
        }
        if loose_root.is_none() && !user_mod_ids.contains(&expectation.id) {
            requirements.push(expectation.content_directory.clone());
        }
        if !remote_overlays.contains(&expectation.id) {
            if let Some(witness) = &expectation.map_witness {
                let maps = if user_mod_ids.contains(&expectation.id) {
                    &all_maps
                } else {
                    &corpus_maps
                };
                if !maps.iter().any(|map| map.path.to_lowercase() == witness.to_lowercase()) {
                    requirements.push(witness.clone());
                }
            }
        }
        let availability = if let Some(reason) = &expectation.unresolved_reason {
            ProductAvailability::Unresolved { reason: reason.clone() }
        } else if requirements.is_empty() {
            ProductAvailability::Installed
        } else {
            ProductAvailability::Missing { requirements }
        };
        let mut product_archives = user_archives.clone();
        product_archives.extend(found.iter().cloned());
        let mut product_diagnostics = diagnostics.clone();
        product_diagnostics.extend(user_diagnostics.iter().cloned());
        products.push(CatalogProduct {
            id,
            expectation: expectation.clone(),
            availability,
            archives: product_archives,
            loose_root: loose_root.clone(),
            user_content: user_root.as_ref().map(|root| UserContent {
                root: root.to_string_lossy().into_owned(),
                archives: user_archives,
            }),
            maps: all_maps,
            diagnostics: product_diagnostics,
        });
    }
    // An unreadable base remains a requirement even when its filenames are present.
    let installed_base: HashMap<String, bool> = products
        .iter()
        .map(|product| {
            (
                product.expectation.id.clone(),
                product.availability == ProductAvailability::Installed,
            )
        })
        .collect();
    let checked: Vec<CatalogProduct> = products
        .into_iter()
        .map(|product| {
            if product.availability != ProductAvailability::Installed {
                return product;
            }
            let Some(base_id) = product.expectation.base_product.clone() else {
                return product;
            };
            let installed = installed_base.get(base_id.as_str()).copied().unwrap_or(false);
            if installed {
                product
            } else {
                CatalogProduct {
                    availability: ProductAvailability::Missing {
                        requirements: vec![format!("base product {base_id}")],
                    },
                    ..product
                }
            }
        })
        .collect();
    InstalledCatalog::new(
        corpus_root.to_string_lossy().into_owned(),
        checked,
        root_archives,
        generation,
        user_content_root.map(|root| root.to_string_lossy().into_owned()),
    )
}

// `launch.ts`.

/// Launch preset: an executable recipe minus its resolved outputs (`LaunchPreset`).
#[derive(Debug, Clone, PartialEq)]
pub struct LaunchPreset {
    /// Recipe identity.
    pub id: RecipeId,
    /// Weapon behaviors.
    pub weapon_behaviors: Vec<ResolvedWeaponBehaviorSelection>,
    /// Gameplay mods.
    pub mods: Vec<ResolvedGameplayMod>,
    /// Map selection.
    pub map: MapSelection,
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
    pub execution: Vec<ExecutionSelection>,
    /// Provider timing.
    pub timing: Vec<ProviderTiming>,
    /// Frame ordering.
    pub ordering: FrameOrdering,
}

/// Selected launch: a preset with the choice that produced it (`SelectedLaunch`).
#[derive(Debug, Clone, PartialEq)]
pub struct SelectedLaunch {
    /// Recipe identity.
    pub id: RecipeId,
    /// Chosen preset.
    pub preset: RecipeId,
    /// Weapon behaviors.
    pub weapon_behaviors: Vec<ResolvedWeaponBehaviorSelection>,
    /// Gameplay mods.
    pub mods: Vec<ResolvedGameplayMod>,
    /// Map selection.
    pub map: MapSelection,
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
    pub execution: Vec<ExecutionSelection>,
    /// Provider timing.
    pub timing: Vec<ProviderTiming>,
    /// Frame ordering.
    pub ordering: FrameOrdering,
}

/// Launch resolution inputs (`ResolveLaunchOptions`).
#[derive(Debug, Clone)]
pub struct ResolveLaunchOptions<'a> {
    /// Launch choice.
    pub choice: &'a LaunchChoice,
    /// Launch preset.
    pub preset: &'a LaunchPreset,
    /// Installed catalog.
    pub catalog: &'a InstalledCatalog,
    /// Recipe identity override (defaults to the preset identity).
    pub id: Option<RecipeId>,
    /// Mount options override.
    pub mounts: Option<OpenMountOptions>,
}

/// Prepared launch: selected decisions plus their mount plan.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedLaunch {
    /// Selected launch.
    pub selected: SelectedLaunch,
    /// Mount plan.
    pub plan: ResolvedMountPlan,
}

/// Resource resolution kind (`"map" | "artifact"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LaunchResourceKind {
    /// Map geometry: any mount in the plan may supply the bytes.
    Map,
    /// Execution artifact: only the content and its base may supply the bytes.
    Artifact,
}

/// QVM role for a compatibility read (donor `QvmRole` at the launch boundary).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QvmCompatRole {
    /// Server game.
    Qagame,
    /// Client game.
    Cgame,
    /// UI.
    Ui,
}

/// Selected-arsenal source adapters (donor `catalog/weapons.ts`).
///
/// The weapons catalog is sibling-owned; launch resolution receives it here so
/// this module stays free of `q1`/`q2` mission-pack dependencies.
pub trait LaunchWeaponSources {
    /// Canonical weapon provider for a map (`canonicalWeaponSource`).
    fn canonical_weapon_source(
        &self,
        map: &ProviderReference,
        weapon: &ProviderReference,
        catalog: &InstalledCatalog,
    ) -> Result<ProviderReference, CatalogError>;
    /// Weapon resources outside the map content (`selectedWeaponResources`).
    fn selected_weapon_resources(
        &self,
        map: &ProviderReference,
        weapons: &[ProviderReference],
        catalog: &InstalledCatalog,
    ) -> Result<Vec<ResourceRequest>, CatalogError>;
    /// Weapon provider timing (`selectedWeaponTiming`).
    fn selected_weapon_timing(
        &self,
        map: &ProviderReference,
        weapons: &[ProviderReference],
        catalog: &InstalledCatalog,
    ) -> Result<Vec<ProviderTiming>, CatalogError>;
    /// Admit one weapon timing row (`admitWeaponTiming`).
    fn admit_weapon_timing(
        &self,
        timing: &mut Vec<ProviderTiming>,
        weapon: &ProviderTiming,
    ) -> Result<(), CatalogError>;
    /// Stock weapon provider identities (`Q1_WEAPON_PROVIDERS`,
    /// `Q1_HIPNOTIC_WEAPON_PROVIDERS`, `Q2_WEAPON_PROVIDERS`).
    fn weapon_provider_ids(&self) -> Vec<ProviderId>;
}

/// QVM compatibility profile reads (donor `compat/qvm/compatibility.ts`).
///
/// The compat lane bridges this to `qa-guest`; launch resolution only needs
/// the selected ABI profile, never the declaration itself.
pub trait LaunchQvmCompatibility {
    /// Read the ABI profile guarding a QVM artifact (`readQvmCompatibility`).
    fn read_qvm_compatibility(
        &self,
        mounts: &dyn BehaviorMounts,
        artifact_path: &str,
        digest: &ContentDigest,
        role: QvmCompatRole,
    ) -> Result<QvmAbiProfile, CatalogError>;
}

/// Minimal mount surface for behavior discovery (`Pick<MountedContent, 'open'>`).
pub trait BehaviorMounts {
    /// Open a resource by path through the ambient order.
    fn open_behavior(&self, path: &str) -> Result<Option<OpenedResource>, CatalogError>;
}

impl BehaviorMounts for MountedContent {
    fn open_behavior(&self, path: &str) -> Result<Option<OpenedResource>, CatalogError> {
        Ok(self.open(path, |_| true)?)
    }
}

impl BehaviorMounts for OrderedReader<'_> {
    fn open_behavior(&self, path: &str) -> Result<Option<OpenedResource>, CatalogError> {
        Ok(self.open(path, |_| true)?)
    }
}

/// Keep the preset decision or use the explicit selection (`selection`).
fn selected<T: Clone>(choice: &LaunchSelection<T>, preset: &T) -> T {
    match choice {
        LaunchSelection::Selected(value) => value.clone(),
        LaunchSelection::Preset => preset.clone(),
    }
}

/// Launch choice that keeps every preset decision (`presetChoice`).
#[must_use]
pub fn preset_choice(preset: RecipeId) -> LaunchChoice {
    LaunchChoice {
        preset,
        map: LaunchSelection::Preset,
        campaign: LaunchSelection::Preset,
        movement: LaunchSelection::Preset,
        character: LaunchSelection::Preset,
        weapons: LaunchSelection::Preset,
        equipment: LaunchSelection::Preset,
        enemies: LaunchSelection::Preset,
        presentation: LaunchSelection::Preset,
        engine_behavior: LaunchSelection::Preset,
        combat: LaunchSelection::Preset,
        inventory: LaunchSelection::Preset,
        r#match: LaunchSelection::Preset,
        transition: LaunchSelection::Preset,
        execution: LaunchSelection::Preset,
    }
}

/// Apply a launch choice to a preset (`selectLaunch`).
///
/// Selecting behavior cannot replace campaign gamecode, movement, character
/// or assets; only explicit selections replace preset decisions.
pub fn select_launch(
    choice: &LaunchChoice,
    preset: &LaunchPreset,
    id: Option<&RecipeId>,
) -> Result<SelectedLaunch, CatalogError> {
    if choice.preset != preset.id {
        return Err(invalid(format!(
            "Requested preset {} does not match supplied preset {}",
            choice.preset, preset.id
        )));
    }
    Ok(SelectedLaunch {
        id: id.cloned().unwrap_or_else(|| preset.id.clone()),
        preset: preset.id.clone(),
        weapon_behaviors: preset.weapon_behaviors.clone(),
        mods: preset.mods.clone(),
        map: selected(&choice.map, &preset.map),
        campaign: selected(&choice.campaign, &preset.campaign),
        movement: selected(&choice.movement, &preset.movement),
        character: selected(&choice.character, &preset.character),
        weapons: selected(&choice.weapons, &preset.weapons),
        equipment: selected(&choice.equipment, &preset.equipment),
        enemies: selected(&choice.enemies, &preset.enemies),
        presentation: selected(&choice.presentation, &preset.presentation),
        engine_behavior: selected(&choice.engine_behavior, &preset.engine_behavior),
        combat: selected(&choice.combat, &preset.combat),
        inventory: selected(&choice.inventory, &preset.inventory),
        r#match: selected(&choice.r#match, &preset.r#match),
        transition: selected(&choice.transition, &preset.transition),
        execution: selected(&choice.execution, &preset.execution),
        timing: preset.timing.clone(),
        ordering: preset.ordering.clone(),
    })
}

/// Mount identity path used for provenance comparisons (`mountPath`).
fn launch_mount_path(mount: &ContentMount) -> &str {
    match mount {
        ContentMount::Archive(mount) => &mount.archive_path,
        ContentMount::Loose(mount) => &mount.root_path,
    }
}

/// Mount identity path behind resolved bytes.
fn provenance_mount_path(provenance: &ResourceProvenance) -> &str {
    match provenance {
        ResourceProvenance::Archive { mount, .. } => &mount.archive_path,
        ResourceProvenance::Loose { mount, .. } => &mount.root_path,
    }
}

/// Content identities a launch requires (`requiredContent`), first-seen order.
fn required_content(launch: &SelectedLaunch) -> Vec<ContentId> {
    let mut references: Vec<&ProviderReference> = vec![
        &launch.map.entities,
        &launch.movement,
        &launch.character.definition,
        &launch.character.appearance,
    ];
    references.extend(launch.weapons.iter());
    let equipment = equipment_provider_refs(&launch.equipment);
    references.extend(equipment.iter());
    references.extend([
        &launch.engine_behavior,
        &launch.combat,
        &launch.inventory,
        &launch.r#match,
        &launch.transition,
        &launch.presentation.hud,
        &launch.presentation.effects,
        &launch.presentation.audio,
    ]);
    let owners: Vec<&ProviderReference> = launch
        .execution
        .iter()
        .map(|module| match module {
            ExecutionModule::Builtin { owner, .. }
            | ExecutionModule::Quakec { owner, .. }
            | ExecutionModule::Qvm { owner, .. }
            | ExecutionModule::Native { owner, .. } => owner,
        })
        .collect();
    references.extend(owners);
    if let CampaignSelection::Campaign { mission, gamecode } = &launch.campaign {
        references.push(mission);
        references.push(gamecode);
    }
    let monsters = selected_monster_definitions(&launch.enemies);
    references.extend(monsters.iter().map(|definition| &definition.source));
    let mut contents: Vec<ContentId> = vec![launch.map.geometry.content.clone(), launch.presentation.assets.clone()];
    if let EnvironmentSelection::Selected { resource } = &launch.presentation.environment {
        contents.push(resource.content.clone());
    }
    contents.extend(references.into_iter().map(|reference| reference.content.clone()));
    contents.extend(launch.execution.iter().filter_map(|module| match module {
        ExecutionModule::Builtin { .. } => None,
        ExecutionModule::Quakec { artifact, .. }
        | ExecutionModule::Qvm { artifact, .. }
        | ExecutionModule::Native { artifact, .. } => Some(artifact.content.clone()),
    }));
    let mut seen: HashSet<ContentId> = HashSet::new();
    contents.into_iter().filter(|id| seen.insert(id.clone())).collect()
}

/// Default order preferring a content and its base (`orderForContent`).
fn order_for_content(
    catalog: &InstalledCatalog,
    plan: &ResolvedMountPlan,
    content: &ContentId,
) -> Result<Vec<MountId>, CatalogError> {
    let first = catalog.mounts_for(content.as_str())?;
    let by_path: HashMap<&str, &MountId> = plan
        .mounts
        .iter()
        .map(|mount| (launch_mount_path(mount), &mount.identity().id))
        .collect();
    let mut seen: HashSet<MountId> = HashSet::new();
    let mut order: Vec<MountId> = Vec::new();
    for mount in &first {
        if let Some(id) = by_path.get(launch_mount_path(mount)) {
            if seen.insert((*id).clone()) {
                order.push((*id).clone());
            }
        }
    }
    for id in &plan.default_order {
        if seen.insert(id.clone()) {
            order.push(id.clone());
        }
    }
    Ok(order)
}

/// Resolve one launch resource through mounted content (`resolveLaunchResource`).
pub fn resolve_launch_resource(
    catalog: &InstalledCatalog,
    mounted: &MountedContent,
    request: &ResourceRequest,
    kind: LaunchResourceKind,
) -> Result<ResolvedResourceReference, CatalogError> {
    let allowed = catalog.mounts_for(request.content.as_str())?;
    let allowed_paths: Vec<&str> = allowed.iter().map(launch_mount_path).collect();
    let opened = match kind {
        LaunchResourceKind::Map => mounted.open(&request.path, |_| true)?,
        LaunchResourceKind::Artifact => {
            let reader = mounted.borrow_ordered_reader(&OrderedPlan {
                id: create_mount_plan_id("artifact", &hex_lower(request.content.as_str().as_bytes()))?,
                default_order: order_for_content(catalog, &mounted.plan, &request.content)?,
                prefix_orders: Vec::new(),
            })?;
            reader.open(&request.path, |mount| allowed_paths.contains(&launch_mount_path(mount)))?
        }
    };
    let Some(opened) = opened else {
        return Err(failed(format!(
            "Required resource is missing: {}/{}",
            request.content, request.path
        )));
    };
    if !allowed_paths.contains(&provenance_mount_path(&opened.reference.provenance)) {
        let kind_name = match kind {
            LaunchResourceKind::Map => "map",
            LaunchResourceKind::Artifact => "artifact",
        };
        return Err(failed(format!(
            "Required {kind_name} is absent from its selected content and base: {}/{}",
            request.content, request.path
        )));
    }
    Ok(opened.reference)
}

/// Donor module-role text for execution conflict keys.
fn module_role_name(role: ModuleRole) -> &'static str {
    match role {
        ModuleRole::ServerGame => "server-game",
        ModuleRole::ClientGame => "client-game",
        ModuleRole::Ui => "ui",
    }
}

/// Validate execution roles and build the launch mount plan (`prepareLaunchMountPlan`).
pub fn prepare_launch_mount_plan(
    options: &ResolveLaunchOptions,
    weapons: &dyn LaunchWeaponSources,
) -> Result<PreparedLaunch, CatalogError> {
    let choice = select_launch(options.choice, options.preset, options.id.as_ref())?;
    let mut canonical_weapons = Vec::with_capacity(choice.weapons.len());
    for weapon in &choice.weapons {
        canonical_weapons.push(weapons.canonical_weapon_source(&choice.map.entities, weapon, options.catalog)?);
    }
    let selected = SelectedLaunch {
        weapons: canonical_weapons,
        ..choice
    };
    validate_equipment(&selected.equipment, options.catalog)?;
    validate_monsters(&selected.enemies, options.catalog)?;
    let required = required_content(&selected);
    for content in &required {
        options.catalog.require(content.as_str())?;
    }
    let mut execution_roles: HashSet<String> = HashSet::new();
    for module in &selected.execution {
        let (owner, role) = match module {
            ExecutionModule::Builtin { owner, role, .. }
            | ExecutionModule::Qvm { owner, role, .. }
            | ExecutionModule::Native { owner, role, .. } => (owner, *role),
            // The donor quakec module carries a constant `server-game` role.
            ExecutionModule::Quakec { owner, .. } => (owner, ModuleRole::ServerGame),
        };
        let key = format!(
            "{}:{}/{}",
            owner.provider.namespace,
            owner.provider.name,
            module_role_name(role)
        );
        if !execution_roles.insert(key.clone()) {
            return Err(failed(format!("Conflicting execution modules for {key}")));
        }
    }
    let base_plan = options.catalog.create_mount_plan(&MountPlanSelection {
        id: create_mount_plan_id("launch", &hex_lower(selected.id.as_str().as_bytes()))?,
        assets: selected.presentation.assets.clone(),
        geometry: selected.map.geometry.content.clone(),
        rules: Some(selected.combat.content.clone()),
        explicit_presentation: matches!(options.choice.presentation, LaunchSelection::Selected(_)),
        additional: required,
    })?;
    // Ordered map: insertion order keeps prefix orders deterministic.
    let mut artifacts: Vec<(String, HashSet<ContentId>)> = Vec::new();
    for module in &selected.execution {
        let artifact = match module {
            ExecutionModule::Builtin { .. } => continue,
            ExecutionModule::Quakec { artifact, .. }
            | ExecutionModule::Qvm { artifact, .. }
            | ExecutionModule::Native { artifact, .. } => artifact,
        };
        let path = normalize_resource_path(&artifact.path)?;
        match artifacts.iter_mut().find(|(prefix, _)| *prefix == path) {
            Some((_, sources)) => {
                sources.insert(artifact.content.clone());
            }
            None => {
                artifacts.push((path, HashSet::from([artifact.content.clone()])));
            }
        }
    }
    let mut artifact_orders: Vec<PrefixMountOrder> = Vec::new();
    for (prefix, sources) in &artifacts {
        if sources.len() != 1 {
            continue;
        }
        for content in sources {
            artifact_orders.push(PrefixMountOrder {
                prefix: prefix.clone(),
                mounts: order_for_content(options.catalog, &base_plan, content)?,
            });
        }
    }
    let mut prefix_orders = artifact_orders;
    prefix_orders.extend(base_plan.prefix_orders.clone());
    Ok(PreparedLaunch {
        selected,
        plan: ResolvedMountPlan {
            prefix_orders,
            ..base_plan
        },
    })
}

/// Admit a resolved resource, replacing same-identity bytes in place.
fn admit_resource(resources: &mut Vec<ResolvedResourceReference>, resource: ResolvedResourceReference) {
    match resources.iter_mut().find(|existing| existing.id == resource.id) {
        Some(existing) => *existing = resource,
        None => resources.push(resource),
    }
}

/// Resolve a launch choice into an executable recipe (`resolveLaunch`).
pub fn resolve_launch(
    options: &ResolveLaunchOptions,
    weapons: &dyn LaunchWeaponSources,
    compat: &dyn LaunchQvmCompatibility,
) -> Result<ExecutableRecipe, CatalogError> {
    let PreparedLaunch { selected, plan } = prepare_launch_mount_plan(options, weapons)?;
    let mounted = open_mount_plan(&plan, options.mounts.clone().unwrap_or_default())?;
    let mut resources: Vec<ResolvedResourceReference> = Vec::new();
    let geometry = resolve_launch_resource(
        options.catalog,
        &mounted,
        &selected.map.geometry,
        LaunchResourceKind::Map,
    )?;
    admit_resource(&mut resources, geometry.clone());
    let groups = [
        (
            "weapon",
            weapons.selected_weapon_resources(&selected.map.entities, &selected.weapons, options.catalog)?,
        ),
        ("equipment", equipment_resources(&selected.equipment)),
        ("monster", monster_resources(&selected.enemies)?),
        (
            "environment",
            match &selected.presentation.environment {
                EnvironmentSelection::Selected { resource } => vec![resource.clone()],
                _ => Vec::new(),
            },
        ),
    ];
    for (kind, requests) in &groups {
        let mut contents: Vec<&ContentId> = Vec::new();
        for request in requests {
            if !contents.contains(&&request.content) {
                contents.push(&request.content);
            }
        }
        for content in contents {
            let order = order_for_content(options.catalog, &mounted.plan, content)?;
            let allowed = options.catalog.mounts_for(content.as_str())?;
            let allowed_paths: Vec<&str> = allowed.iter().map(launch_mount_path).collect();
            let scoped = mounted.borrow_ordered_reader(&OrderedPlan {
                id: create_mount_plan_id(kind, &hex_lower(content.as_str().as_bytes()))?,
                default_order: order,
                prefix_orders: Vec::new(),
            })?;
            for request in requests.iter().filter(|request| &request.content == content) {
                match scoped.resolve(&request.path)? {
                    Some(resource) if allowed_paths.contains(&provenance_mount_path(&resource.provenance)) => {
                        admit_resource(&mut resources, resource);
                    }
                    _ => {
                        return Err(failed(format!(
                            "Required {kind} resource is absent from its selected content and base: {}/{}",
                            content, request.path
                        )));
                    }
                }
            }
        }
    }
    let mut execution: Vec<ResolvedExecutionModule> = Vec::new();
    for module in &selected.execution {
        match module {
            ExecutionModule::Builtin {
                owner,
                implementation,
                role,
                api,
            } => execution.push(ExecutionModule::Builtin {
                owner: owner.clone(),
                implementation: implementation.clone(),
                role: *role,
                api: *api,
            }),
            ExecutionModule::Quakec { owner, artifact, api } => {
                let resolved =
                    resolve_launch_resource(options.catalog, &mounted, artifact, LaunchResourceKind::Artifact)?;
                admit_resource(&mut resources, resolved.clone());
                execution.push(ExecutionModule::Quakec {
                    owner: owner.clone(),
                    artifact: resolved,
                    api: *api,
                });
            }
            ExecutionModule::Qvm {
                owner,
                artifact,
                role,
                api,
            } => {
                let resolved =
                    resolve_launch_resource(options.catalog, &mounted, artifact, LaunchResourceKind::Artifact)?;
                admit_resource(&mut resources, resolved.clone());
                let order = order_for_content(options.catalog, &mounted.plan, &artifact.content)?;
                let scoped = mounted.borrow_ordered_reader(&OrderedPlan {
                    id: create_mount_plan_id("qvm-abi", &hex_lower(artifact.content.as_str().as_bytes()))?,
                    default_order: order,
                    prefix_orders: Vec::new(),
                })?;
                let compat_role = match api {
                    Q3ApiIdentity::Qagame(_) => QvmCompatRole::Qagame,
                    Q3ApiIdentity::Cgame(_) => QvmCompatRole::Cgame,
                    Q3ApiIdentity::Ui(_) => QvmCompatRole::Ui,
                };
                let profile =
                    compat.read_qvm_compatibility(&scoped, &resolved.requested_path, &resolved.digest, compat_role)?;
                let modern = profile == QvmAbiProfile::Modern;
                match role {
                    ModuleRole::ServerGame => execution.push(ExecutionModule::Qvm {
                        owner: owner.clone(),
                        artifact: resolved,
                        role: *role,
                        api: Q3ApiIdentity::Qagame(if modern { 8 } else { 7 }),
                    }),
                    ModuleRole::ClientGame => execution.push(ExecutionModule::Qvm {
                        owner: owner.clone(),
                        artifact: resolved,
                        role: *role,
                        api: Q3ApiIdentity::Cgame(if modern { 4 } else { 3 }),
                    }),
                    ModuleRole::Ui => {
                        let version = match api {
                            Q3ApiIdentity::Qagame(version)
                            | Q3ApiIdentity::Cgame(version)
                            | Q3ApiIdentity::Ui(version) => *version,
                        };
                        execution.push(ExecutionModule::Qvm {
                            owner: owner.clone(),
                            artifact: resolved,
                            role: *role,
                            api: Q3ApiIdentity::Ui(if modern { version } else { 4 }),
                        });
                    }
                }
            }
            ExecutionModule::Native {
                owner,
                artifact,
                profile,
                role,
                api,
            } => {
                let resolved =
                    resolve_launch_resource(options.catalog, &mounted, artifact, LaunchResourceKind::Artifact)?;
                admit_resource(&mut resources, resolved.clone());
                execution.push(ExecutionModule::Native {
                    owner: owner.clone(),
                    artifact: resolved,
                    profile: *profile,
                    role: *role,
                    api: *api,
                });
            }
        }
    }
    let equipment = equipment_providers();
    let mut selected_source_ids: HashSet<ProviderId> = HashSet::new();
    for id in [
        &equipment.threewave,
        &equipment.ctf,
        &equipment.lmctf,
        &equipment.hand_grenades,
    ] {
        selected_source_ids.insert(id.clone());
    }
    for reference in equipment_provider_refs(&selected.equipment) {
        selected_source_ids.insert(reference.provider.clone());
    }
    for id in weapons.weapon_provider_ids() {
        selected_source_ids.insert(id);
    }
    for source in monster_sources() {
        selected_source_ids.insert(source.provider.clone());
    }
    let monster_profiles = selected_monster_timing(&selected.enemies);
    let weapon_profiles = weapons.selected_weapon_timing(&selected.map.entities, &selected.weapons, options.catalog)?;
    let mut timing: Vec<ProviderTiming> = selected
        .timing
        .iter()
        .filter(|entry| !selected_source_ids.contains(&entry.provider))
        .cloned()
        .collect();
    timing.extend(equipment_timing(&selected.equipment));
    timing.extend(monster_profiles.iter().cloned());
    for profile in &weapon_profiles {
        weapons.admit_weapon_timing(&mut timing, profile)?;
    }
    let mut selected_source_order: Vec<ProviderId> = equipment_provider_refs(&selected.equipment)
        .into_iter()
        .map(|reference| reference.provider)
        .collect();
    selected_source_order.extend(monster_profiles.iter().map(|profile| profile.provider.clone()));
    let ordering = match &selected.ordering {
        FrameOrdering::Mixed { providers } => {
            let mut existing: Vec<ProviderId> = providers
                .iter()
                .filter(|provider| !selected_source_ids.contains(*provider))
                .cloned()
                .collect();
            existing.extend(selected_source_order.iter().cloned());
            let mut merged = existing.clone();
            for provider in weapon_profiles
                .iter()
                .map(|profile| &profile.provider)
                .filter(|provider| !existing.contains(provider))
            {
                merged.push(provider.clone());
            }
            FrameOrdering::Mixed { providers: merged }
        }
        FrameOrdering::Native { .. } if weapon_profiles.is_empty() => selected.ordering.clone(),
        FrameOrdering::Native { .. } => {
            let mut seen: HashSet<ProviderId> = HashSet::new();
            let mut providers: Vec<ProviderId> = Vec::new();
            let candidates = [selected.map.entities.provider.clone()]
                .into_iter()
                .chain(selected.timing.iter().map(|entry| entry.provider.clone()))
                .chain(selected_source_order)
                .chain(weapon_profiles.iter().map(|profile| profile.provider.clone()));
            for provider in candidates {
                if seen.insert(provider.clone()) {
                    providers.push(provider);
                }
            }
            FrameOrdering::Mixed { providers }
        }
    };
    Ok(ExecutableRecipe {
        weapon_behaviors: selected.weapon_behaviors.clone(),
        mods: selected.mods.clone(),
        schema_version: 3,
        id: selected.id.clone(),
        preset: selected.preset.clone(),
        map: ResolvedMap {
            geometry_content: selected.map.geometry.content.clone(),
            geometry,
            entities: selected.map.entities.clone(),
        },
        campaign: selected.campaign.clone(),
        movement: selected.movement.clone(),
        character: selected.character.clone(),
        weapons: selected.weapons.clone(),
        equipment: selected.equipment.clone(),
        enemies: selected.enemies.clone(),
        presentation: selected.presentation.clone(),
        engine_behavior: selected.engine_behavior.clone(),
        combat: selected.combat.clone(),
        inventory: selected.inventory.clone(),
        r#match: selected.r#match.clone(),
        transition: selected.transition.clone(),
        execution,
        mounts: mounted.plan.clone(),
        resources,
        timing,
        ordering,
    })
}

// `weapon-behaviors.ts`.

/// Source weapon behavior metadata from a mounted declaration or observed
/// source binding (`SourceWeaponBehaviorMetadata`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceWeaponBehaviorMetadata {
    /// Behavior identity (`namespace:name`).
    pub id: String,
    /// Title.
    pub title: String,
    /// Artifact digest the metadata belongs to.
    pub artifact_digest: ContentDigest,
    /// Projectile role.
    pub role: ProjectileRole,
    /// Fire entrypoint name.
    pub fire_function: String,
    /// Activation gate entrypoint name, when declared.
    pub activation_function: Option<String>,
}

/// Weapon behavior compatibility (`WeaponBehaviorCompatibility`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[allow(clippy::large_enum_variant)]
pub enum WeaponBehaviorCompatibility {
    /// Supported behavior with its definition.
    Supported {
        /// Behavior definition.
        definition: WeaponBehaviorDefinition,
    },
    /// Unsupported behavior with its reason.
    Unsupported {
        /// Reason.
        reason: String,
    },
}

/// QuakeC function row needed for behavior resolution.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QcWeaponFunction {
    /// Function index.
    pub index: u32,
    /// Function name.
    pub name: String,
    /// First statement, or negative when the function has no body.
    pub first_statement: i64,
    /// Parameter word count.
    pub parameter_words: usize,
}

/// QuakeC statement opcodes relevant to think-store inspection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QcWeaponOpcode {
    /// Address-of-field.
    Address,
    /// Store function pointer field.
    StorePFn,
    /// Store function global.
    StoreFn,
    /// Any other opcode.
    Other,
}

/// QuakeC statement row needed for think-store inspection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QcWeaponStatement {
    /// Opcode.
    pub opcode: QcWeaponOpcode,
    /// First operand.
    pub a: i32,
    /// Second operand.
    pub b: i32,
    /// Third operand.
    pub c: i32,
}

/// QuakeC `function` global needed for literal-callback checks.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QcFunctionGlobal {
    /// Global name.
    pub name: String,
    /// Global word offset.
    pub offset: i32,
}

/// QuakeC program surface for weapon behavior resolution.
///
/// The guest lane builds this snapshot from its `QcProgram` (including the
/// `qcWeaponBehaviorCapabilityError` verdict); behavior resolution itself is
/// pure over the snapshot so this module never depends on `qa-guest`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QcWeaponProgramSnapshot {
    /// Program digest.
    pub digest: ContentDigest,
    /// Capability verdict, or `None` when the program can host behaviors.
    pub capability_error: Option<String>,
    /// Program functions in stored order.
    pub functions: Vec<QcWeaponFunction>,
    /// Program statements in stored order.
    pub statements: Vec<QcWeaponStatement>,
    /// Word offset of the `think` field, when the program declares one.
    pub think_field_offset: Option<i32>,
    /// Initial global words, word-addressable.
    pub initial_global_words: Vec<i32>,
    /// `function`-typed globals by name and offset.
    pub function_globals: Vec<QcFunctionGlobal>,
}

impl QcWeaponProgramSnapshot {
    /// Read one initial global word, or `None` when out of range.
    fn word(&self, offset: i32) -> Option<i32> {
        usize::try_from(offset)
            .ok()
            .and_then(|index| self.initial_global_words.get(index).copied())
    }
}

/// Resolve one QuakeC weapon behavior against its source program
/// (`resolveQcWeaponBehavior`).
pub fn resolve_qc_weapon_behavior(
    module: &ModuleIdentity,
    program: &QcWeaponProgramSnapshot,
    metadata: &SourceWeaponBehaviorMetadata,
) -> WeaponBehaviorCompatibility {
    if module.digest != program.digest || metadata.artifact_digest != program.digest {
        return WeaponBehaviorCompatibility::Unsupported {
            reason: "Behavior metadata belongs to a different source artifact".to_string(),
        };
    }
    if let Some(reason) = &program.capability_error {
        return WeaponBehaviorCompatibility::Unsupported { reason: reason.clone() };
    }
    let callback = |name: &str| -> Option<WeaponBehaviorCallback> {
        let function = program.functions.iter().find(|candidate| candidate.name == name)?;
        if function.index == 0 || function.first_statement < 0 || function.parameter_words != 0 {
            return None;
        }
        Some(WeaponBehaviorCallback::Quakec {
            module: module.clone(),
            function_index: function.index,
        })
    };
    let fire = callback(&metadata.fire_function);
    let activate = metadata.activation_function.as_deref().map(callback);
    match (fire, &metadata.activation_function, activate) {
        (Some(fire), None, None) => WeaponBehaviorCompatibility::Supported {
            definition: WeaponBehaviorDefinition {
                id: metadata.id.clone(),
                title: metadata.title.clone(),
                module: module.clone(),
                role: metadata.role,
                activate: None,
                fire,
            },
        },
        (Some(fire), Some(_), Some(Some(activate))) => WeaponBehaviorCompatibility::Supported {
            definition: WeaponBehaviorDefinition {
                id: metadata.id.clone(),
                title: metadata.title.clone(),
                module: module.clone(),
                role: metadata.role,
                activate: Some(activate),
                fire,
            },
        },
        _ => WeaponBehaviorCompatibility::Unsupported {
            reason: "Declared behavior entrypoint is absent from the source program".to_string(),
        },
    }
}

/// Weapon behavior catalog (`WeaponBehaviorCatalog`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WeaponBehaviorCatalog {
    entries: Vec<WeaponBehaviorDefinition>,
}

impl WeaponBehaviorCatalog {
    /// Empty catalog.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a definition, rejecting duplicates.
    pub fn add(&mut self, definition: WeaponBehaviorDefinition) -> Result<(), CatalogError> {
        if self.entries.iter().any(|entry| entry.id == definition.id) {
            return Err(failed(format!("Duplicate weapon behavior {}", definition.id)));
        }
        self.entries.push(definition);
        Ok(())
    }

    /// Definitions for one projectile role.
    #[must_use]
    pub fn for_role(&self, role: ProjectileRole) -> Vec<WeaponBehaviorDefinition> {
        self.entries
            .iter()
            .filter(|entry| entry.role == role)
            .cloned()
            .collect()
    }

    /// Require one definition by identity.
    pub fn require(&self, id: &str) -> Result<WeaponBehaviorDefinition, CatalogError> {
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
            .ok_or_else(|| failed(format!("Unknown weapon behavior {id}")))
    }
}

/// Explicit think-field store between two source functions
/// (`QcTrajectoryBindingInspection`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QcTrajectoryBindingInspection {
    /// Producer function index.
    pub producer_function: u32,
    /// Think function index.
    pub think_function: u32,
    /// Store statement index.
    pub statement: usize,
}

/// Report explicit bytecode stores to the source ABI think field
/// (`inspectQcTrajectoryBindings`); this does not infer a weapon role.
pub fn inspect_qc_trajectory_bindings(program: &QcWeaponProgramSnapshot) -> Vec<QcTrajectoryBindingInspection> {
    let Some(think) = program.think_field_offset else {
        return Vec::new();
    };
    let mut functions: Vec<&QcWeaponFunction> = program
        .functions
        .iter()
        .filter(|function| function.index != 0 && function.first_statement >= 0)
        .collect();
    functions.sort_by_key(|function| function.first_statement);
    let mut result: Vec<QcTrajectoryBindingInspection> = Vec::new();
    for (index, function) in functions.iter().enumerate() {
        let Ok(start) = usize::try_from(function.first_statement) else {
            continue;
        };
        let end = functions
            .get(index + 1)
            .and_then(|next| usize::try_from(next.first_statement).ok())
            .unwrap_or(program.statements.len());
        let mut cursor = start;
        while cursor.saturating_add(1) < end {
            let (Some(address), Some(store)) = (program.statements.get(cursor), program.statements.get(cursor + 1))
            else {
                cursor += 1;
                continue;
            };
            cursor += 1;
            if address.opcode != QcWeaponOpcode::Address
                || store.opcode != QcWeaponOpcode::StorePFn
                || address.c != store.b
                || program.word(address.b) != Some(think)
            {
                continue;
            }
            let target = program.word(store.a);
            let callback = target.and_then(|index| {
                usize::try_from(index)
                    .ok()
                    .and_then(|at| program.functions.get(at))
                    .filter(|candidate| candidate.index != 0 && candidate.first_statement >= 0)
            });
            let Some(callback) = callback else {
                continue;
            };
            // Only the literal function global is an artifact identity;
            // mutable function variables require runtime observation.
            if !program
                .function_globals
                .iter()
                .any(|global| global.offset == store.a && global.name == callback.name)
            {
                continue;
            }
            if program
                .statements
                .iter()
                .any(|statement| statement.opcode == QcWeaponOpcode::StoreFn && statement.b == store.a)
            {
                continue;
            }
            result.push(QcTrajectoryBindingInspection {
                producer_function: function.index,
                think_function: callback.index,
                statement: cursor,
            });
        }
    }
    result
}

/// Mounted weapon behavior discovery (`MountedWeaponBehaviorDiscovery`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MountedWeaponBehaviorDiscovery {
    /// Authored declaration with per-entry compatibility.
    Declared {
        /// Per-entry compatibility verdicts.
        declarations: Vec<WeaponBehaviorCompatibility>,
    },
    /// No declaration; bytecode stores without established roles.
    Undeclared {
        /// Observed think stores.
        bindings: Vec<QcTrajectoryBindingInspection>,
        /// Reason no declaration applies.
        reason: String,
    },
}

/// Whether text is a `namespace:name` behavior identity.
fn is_behavior_id(value: &str) -> bool {
    match value.find(':') {
        Some(colon) => {
            let (namespace, rest) = value.split_at(colon);
            let name = &rest[1..];
            !namespace.is_empty()
                && !namespace.chars().any(char::is_whitespace)
                && !name.is_empty()
                && !name.chars().any(char::is_whitespace)
        }
        None => false,
    }
}

/// Parse a projectile role name.
fn parse_projectile_role(value: &SaveJson) -> Option<ProjectileRole> {
    match value {
        SaveJson::String(role) => match role.as_str() {
            "rocket" => Some(ProjectileRole::Rocket),
            "grenade" => Some(ProjectileRole::Grenade),
            "nail" => Some(ProjectileRole::Nail),
            "bolt" => Some(ProjectileRole::Bolt),
            "plasma" => Some(ProjectileRole::Plasma),
            "energy" => Some(ProjectileRole::Energy),
            "grapple" => Some(ProjectileRole::Grapple),
            _ => None,
        },
        _ => None,
    }
}

/// Parse a declaration digest: branded digests pass through, bare lowercase
/// hex gains its brand.
fn declaration_digest(value: &str) -> Option<ContentDigest> {
    if is_content_digest(value) {
        return Some(ContentDigest(value.to_string()));
    }
    let hex = value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase());
    if hex {
        create_content_digest(value).ok()
    } else {
        None
    }
}

/// Read a mounted provider declaration; artifact identity is checked before
/// any source callback is exposed (`discoverQcWeaponBehaviors`).
pub fn discover_qc_weapon_behaviors(
    mounts: &dyn BehaviorMounts,
    module: &ModuleIdentity,
    program: &QcWeaponProgramSnapshot,
) -> Result<MountedWeaponBehaviorDiscovery, CatalogError> {
    let Some(opened) = mounts.open_behavior("weapon-behaviors.json")? else {
        return Ok(MountedWeaponBehaviorDiscovery::Undeclared {
            bindings: inspect_qc_trajectory_bindings(program),
            reason: "No authored weapon behavior declaration; bytecode callback stores do not establish projectile role, activation gate or selected aspect".to_string(),
        });
    };
    let text = std::str::from_utf8(&opened.bytes).map_err(|_| invalid("Invalid weapon behavior declaration"))?;
    let value = parse_save_json(text).map_err(|_| invalid("Invalid weapon behavior declaration"))?;
    let version_ok = matches!(value.get("version"), Some(SaveJson::Number(version)) if *version == 1.0);
    let SaveJson::Array(behaviors) = value.get("behaviors").cloned().unwrap_or(SaveJson::Null) else {
        return Err(invalid("Invalid weapon behavior declaration"));
    };
    if !version_ok || !matches!(value, SaveJson::Object(_)) {
        return Err(invalid("Invalid weapon behavior declaration"));
    }
    let mut declarations: Vec<WeaponBehaviorCompatibility> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for entry in &behaviors {
        let valid = matches!(entry, SaveJson::Object(_))
            && matches!(entry.get("id"), Some(SaveJson::String(id)) if is_behavior_id(id))
            && matches!(entry.get("title"), Some(SaveJson::String(_)))
            && matches!(entry.get("artifactDigest"), Some(SaveJson::String(_)))
            && matches!(entry.get("fireFunction"), Some(SaveJson::String(_)))
            && entry
                .get("activationFunction")
                .is_none_or(|value| matches!(value, SaveJson::String(_)))
            && entry.get("role").is_some_and(parse_projectile_role_is_valid)
            && matches!(entry.get("aspect"), Some(SaveJson::String(aspect)) if aspect == "trajectory");
        if !valid {
            return Err(invalid("Invalid weapon behavior entry"));
        }
        let (
            Some(SaveJson::String(id)),
            Some(SaveJson::String(title)),
            Some(SaveJson::String(digest_text)),
            Some(SaveJson::String(fire)),
        ) = (
            entry.get("id"),
            entry.get("title"),
            entry.get("artifactDigest"),
            entry.get("fireFunction"),
        )
        else {
            return Err(invalid("Invalid weapon behavior entry"));
        };
        let Some(artifact_digest) = declaration_digest(digest_text) else {
            return Err(invalid("Invalid weapon behavior artifact digest"));
        };
        if !seen.insert(id.clone()) {
            return Err(invalid(format!("Duplicate declared weapon behavior {id}")));
        }
        let role = entry
            .get("role")
            .and_then(parse_projectile_role)
            .ok_or_else(|| invalid("Invalid weapon behavior entry"))?;
        let activation = match entry.get("activationFunction") {
            Some(SaveJson::String(name)) => Some(name.clone()),
            _ => None,
        };
        declarations.push(resolve_qc_weapon_behavior(
            module,
            program,
            &SourceWeaponBehaviorMetadata {
                id: id.clone(),
                title: title.clone(),
                artifact_digest,
                role,
                fire_function: fire.clone(),
                activation_function: activation,
            },
        ));
    }
    Ok(MountedWeaponBehaviorDiscovery::Declared { declarations })
}

/// Whether a JSON value names a projectile role.
fn parse_projectile_role_is_valid(value: &SaveJson) -> bool {
    parse_projectile_role(value).is_some()
}

// `qvm-weapon-behaviors.ts`.

/// Mounted QVM weapon behavior (`MountedQvmWeaponBehavior`).
///
/// The artifact and profile types live in `qa-guest`, above this crate, so
/// callers carry their own representations through the service trait.
#[derive(Debug, Clone, PartialEq)]
pub struct MountedQvmWeaponBehavior<Artifact, Profile> {
    /// Resolved bytecode artifact.
    pub artifact: Artifact,
    /// Artifact resource reference.
    pub resource: ResolvedResourceReference,
    /// Weapon profile.
    pub profile: Profile,
    /// Raw declaration value.
    pub declaration: SaveJson,
}

/// QVM artifact resolution outcome at the behavior boundary.
#[derive(Debug, Clone, PartialEq)]
pub enum QvmWeaponArtifactResolution<Artifact> {
    /// Actual bytecode.
    Bytecode(Artifact),
    /// Any other artifact kind.
    Other,
}

/// QVM weapon artifact and profile reads (donor `compat/qvm/artifacts.ts` and
/// `compat/qvm/weapon-behavior-profile.ts`, bridged by the guest lane).
pub trait QvmWeaponBehaviorService<Artifact, Profile> {
    /// Resolve a `qagame` artifact from mounted bytes (`resolveQvmArtifact`).
    fn resolve_qvm_artifact(
        &self,
        abi_profile: QvmAbiProfile,
        bytes: &[u8],
        module: &ModuleIdentity,
    ) -> Result<QvmWeaponArtifactResolution<Artifact>, CatalogError>;
    /// Read the weapon profile guarding a bytecode artifact
    /// (`readQvmWeaponProfile`).
    fn read_qvm_weapon_profile(&self, declaration: &SaveJson, artifact: &Artifact) -> Result<Profile, CatalogError>;
    /// Behavior identity carried by a profile.
    fn qvm_weapon_profile_id(&self, profile: &Profile) -> String;
}

/// Read a QVM weapon behavior document (`readQvmWeaponBehaviorDocument`).
pub fn read_qvm_weapon_behavior_document(bytes: &[u8]) -> Result<Vec<SaveJson>, CatalogError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| CatalogError::from(save_error("qvm-weapon-behaviors.json", "expected UTF-8")))?;
    let value = parse_save_json(text)?;
    let reader = SaveReader::at(&value, "qvm-weapon-behaviors.json");
    reader.field("version").literal_i64(1)?;
    Ok(reader
        .field("profiles")
        .list(|entry| Ok::<SaveJson, ValueError>(entry.value.cloned().unwrap_or(SaveJson::Null)))?)
}

/// Load one declared QVM weapon behavior (`loadQvmWeaponBehavior`).
pub fn load_qvm_weapon_behavior<Artifact, Profile>(
    mounts: &dyn BehaviorMounts,
    provider: &ProviderId,
    declaration: &SaveJson,
    service: &dyn QvmWeaponBehaviorService<Artifact, Profile>,
) -> Result<MountedQvmWeaponBehavior<Artifact, Profile>, CatalogError> {
    let reader = SaveReader::new(declaration);
    let path = normalize_resource_path(&reader.field("artifactPath").string()?)?;
    let abi_profile = match reader
        .field("abiProfile")
        .choice_str(&["q3-modern", "q3-1.16n-base"])?
        .as_str()
    {
        "q3-modern" => QvmAbiProfile::Modern,
        _ => QvmAbiProfile::Legacy116n,
    };
    let Some(opened) = mounts.open_behavior(&path)? else {
        return Err(failed(format!("Declared QVM behavior artifact is missing: {path}")));
    };
    let module = module_identity_for_opened(provider, path, &opened);
    let artifact = match service.resolve_qvm_artifact(abi_profile, &opened.bytes, &module)? {
        QvmWeaponArtifactResolution::Bytecode(artifact) => artifact,
        QvmWeaponArtifactResolution::Other => {
            return Err(failed("QVM weapon behavior requires actual bytecode"));
        }
    };
    let profile = service.read_qvm_weapon_profile(declaration, &artifact)?;
    Ok(MountedQvmWeaponBehavior {
        artifact,
        resource: opened.reference,
        profile,
        declaration: declaration.clone(),
    })
}

/// Discover mounted QVM weapon behaviors (`discoverQvmWeaponBehaviors`).
pub fn discover_qvm_weapon_behaviors<Artifact, Profile>(
    mounts: &dyn BehaviorMounts,
    provider: &ProviderId,
    service: &dyn QvmWeaponBehaviorService<Artifact, Profile>,
) -> Result<Option<Vec<MountedQvmWeaponBehavior<Artifact, Profile>>>, CatalogError> {
    let Some(opened) = mounts.open_behavior("qvm-weapon-behaviors.json")? else {
        return Ok(None);
    };
    let mut result: Vec<MountedQvmWeaponBehavior<Artifact, Profile>> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for declaration in read_qvm_weapon_behavior_document(&opened.bytes)? {
        let entry = load_qvm_weapon_behavior(mounts, provider, &declaration, service)?;
        let id = service.qvm_weapon_profile_id(&entry.profile);
        if !seen.insert(id.clone()) {
            return Err(failed(format!("Duplicate QVM weapon behavior {id}")));
        }
        result.push(entry);
    }
    Ok(Some(result))
}

// `native-weapon-behaviors.ts`.

/// Mounted native weapon behavior (`MountedNativeWeaponBehavior`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountedNativeWeaponBehavior {
    /// Artifact resource reference.
    pub resource: ResolvedResourceReference,
    /// Behavior definition.
    pub definition: WeaponBehaviorDefinition,
    /// Behavior declaration.
    pub declaration: NativeWeaponBehaviorDeclaration,
}

/// Native weapon declaration and image reads (donor
/// `compat/q2/rerelease/native-weapon-declaration.ts` and
/// `compat/q2/rerelease/weapon-behavior-profile.ts`, bridged by the compat
/// lane).
pub trait NativeWeaponBehaviorService {
    /// Read a native weapon declaration (`readNativeWeaponDeclaration`).
    fn read_native_weapon_declaration(&self, value: &SaveJson)
        -> Result<NativeWeaponBehaviorDeclaration, CatalogError>;
    /// Revalidate a declaration against its module, validate the executable
    /// image, and derive the executable definition (`readNativeWeaponDeclaration`
    /// with module identity, `validateNativeWeaponImage`,
    /// `rereleaseWeaponDefinition`).
    fn native_weapon_definition(
        &self,
        declaration: &NativeWeaponBehaviorDeclaration,
        module: &ModuleIdentity,
        image_bytes: &[u8],
    ) -> Result<Option<WeaponBehaviorDefinition>, CatalogError>;
    /// Built-in rerelease declaration for a stock game image
    /// (`builtInRereleaseWeaponDeclaration`).
    fn builtin_rerelease_weapon_declaration(
        &self,
        module: &ModuleIdentity,
    ) -> Result<Option<NativeWeaponBehaviorDeclaration>, CatalogError>;
}

/// Read a native weapon behavior document (`readNativeWeaponBehaviorDocument`).
pub fn read_native_weapon_behavior_document(bytes: &[u8]) -> Result<Vec<SaveJson>, CatalogError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| CatalogError::from(save_error("native-weapon-behaviors.json", "expected UTF-8")))?;
    let value = parse_save_json(text)?;
    let reader = SaveReader::at(&value, "native-weapon-behaviors.json");
    reader.field("version").literal_i64(1)?;
    Ok(reader
        .field("profiles")
        .list(|entry| Ok::<SaveJson, ValueError>(entry.value.cloned().unwrap_or(SaveJson::Null)))?)
}

/// Load one declared native weapon behavior (`loadNativeWeaponBehavior`).
pub fn load_native_weapon_behavior(
    mounts: &dyn BehaviorMounts,
    provider: &ProviderId,
    value: &SaveJson,
    service: &dyn NativeWeaponBehaviorService,
) -> Result<MountedNativeWeaponBehavior, CatalogError> {
    let declaration = service.read_native_weapon_declaration(value)?;
    load_declared_native_weapon_behavior(mounts, provider, &declaration, service)
}

/// Load a native weapon behavior from its declaration.
fn load_declared_native_weapon_behavior(
    mounts: &dyn BehaviorMounts,
    provider: &ProviderId,
    declaration: &NativeWeaponBehaviorDeclaration,
    service: &dyn NativeWeaponBehaviorService,
) -> Result<MountedNativeWeaponBehavior, CatalogError> {
    let Some(opened) = mounts.open_behavior(&declaration.artifact_path)? else {
        return Err(failed(format!(
            "Declared native behavior artifact is missing: {}",
            declaration.artifact_path
        )));
    };
    let module = module_identity_for_opened(provider, opened.reference.requested_path.clone(), &opened);
    let Some(definition) = service.native_weapon_definition(declaration, &module, &opened.bytes)? else {
        return Err(failed("Declared native behavior has no executable definition"));
    };
    Ok(MountedNativeWeaponBehavior {
        resource: opened.reference,
        definition,
        declaration: declaration.clone(),
    })
}

/// Module identity for an opened behavior artifact: the true content digest,
/// forced once and cached on the opened resource.
fn module_identity_for_opened(provider: &ProviderId, artifact_path: String, opened: &OpenedResource) -> ModuleIdentity {
    ModuleIdentity {
        id: provider.clone(),
        artifact_path,
        digest: opened.content_digest().clone(),
        revision: opened.content_digest().as_str().to_string(),
    }
}

/// Discover mounted native weapon behaviors (`discoverNativeWeaponBehaviors`).
pub fn discover_native_weapon_behaviors(
    mounts: &dyn BehaviorMounts,
    provider: &ProviderId,
    service: &dyn NativeWeaponBehaviorService,
) -> Result<Option<Vec<MountedNativeWeaponBehavior>>, CatalogError> {
    let Some(document) = mounts.open_behavior("native-weapon-behaviors.json")? else {
        let Some(opened) = mounts.open_behavior("game_x64.dll")? else {
            return Ok(None);
        };
        let module = module_identity_for_opened(provider, opened.reference.requested_path.clone(), &opened);
        let Some(declaration) = service.builtin_rerelease_weapon_declaration(&module)? else {
            return Ok(None);
        };
        return Ok(Some(vec![load_declared_native_weapon_behavior(
            mounts,
            provider,
            &declaration,
            service,
        )?]));
    };
    let mut result: Vec<MountedNativeWeaponBehavior> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for declaration in read_native_weapon_behavior_document(&document.bytes)? {
        let entry = load_native_weapon_behavior(mounts, provider, &declaration, service)?;
        if !seen.insert(entry.definition.id.clone()) {
            return Err(failed(format!(
                "Duplicate native weapon behavior {}",
                entry.definition.id
            )));
        }
        result.push(entry);
    }
    Ok(Some(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{
        ModuleIdentity, QvmAbiProfile, QvmGrappleCallbacks, QvmGrappleDefinition, QvmGrappleFields,
        QvmGrappleFovOffset, QvmGrappleGlobals, QvmGrappleMovement, QvmGrapplePresentation, QvmGrappleViewAnchor,
        QvmGrappleViewAttachment, ResourceResolution, SourceModuleApi,
    };
    use crate::monsters::{MonsterFamily, MonsterProgram};
    use qa_core::math::Vec3;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_root(name: &str) -> PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("qa-catalog-{name}-{}-{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn expectation(
        id: &str,
        family: GameFamily,
        edition: &str,
        campaign: &str,
        base: Option<&str>,
    ) -> ProductExpectation {
        ProductExpectation {
            id: id.to_string(),
            family,
            edition: edition.to_string(),
            campaign: campaign.to_string(),
            title: id.to_string(),
            content_directory: format!("{}/{campaign}", family_name(family)),
            base_product: base.map(str::to_string),
            required_content_archives: Vec::new(),
            required_programs: Vec::new(),
            map_witness: None,
            unresolved_reason: None,
        }
    }

    fn installed_product(
        id: &str,
        family: GameFamily,
        edition: &str,
        campaign: &str,
        base: Option<&str>,
    ) -> CatalogProduct {
        let content = create_content_id(&ContentIdentity {
            family,
            edition: edition.to_string(),
            package: campaign.to_string(),
            revision: "installed".to_string(),
        })
        .unwrap();
        CatalogProduct {
            id: content,
            expectation: expectation(id, family, edition, campaign, base),
            availability: ProductAvailability::Installed,
            archives: Vec::new(),
            loose_root: None,
            user_content: None,
            maps: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn test_catalog(products: Vec<CatalogProduct>) -> InstalledCatalog {
        InstalledCatalog::new("/corpus".to_string(), products, Vec::new(), 0, None).unwrap()
    }

    fn build_pak(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut payloads = Vec::new();
        let mut dir = Vec::new();
        let mut offset = 12u32;
        for (name, bytes) in entries {
            let mut raw = [0u8; 56];
            raw[..name.len()].copy_from_slice(name.as_bytes());
            dir.extend_from_slice(&raw);
            dir.extend_from_slice(&offset.to_le_bytes());
            dir.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            payloads.extend_from_slice(bytes);
            offset += bytes.len() as u32;
        }
        let mut data = Vec::new();
        data.extend_from_slice(b"PACK");
        data.extend_from_slice(&offset.to_le_bytes());
        data.extend_from_slice(&(dir.len() as u32).to_le_bytes());
        data.extend_from_slice(&payloads);
        data.extend_from_slice(&dir);
        data
    }

    #[test]
    fn stock_products_cover_all_families() {
        let products = expected_products();
        assert_eq!(products.len(), 27);
        let ctf = products.iter().find(|product| product.id == "q1-classic-ctf").unwrap();
        assert_eq!(ctf.map_witness.as_deref(), Some("maps/ctfstart.bsp"));
        assert_eq!(ctf.base_product.as_deref(), Some("q1-classic-id1"));
        let n64 = products
            .iter()
            .find(|product| product.id == "q2-rerelease-n64")
            .unwrap();
        assert_eq!(n64.map_witness.as_deref(), Some("maps/q64/rtest.bsp"));
        assert_eq!(n64.content_directory, "q2/rerelease/baseq2");
        let demo = products.iter().find(|product| product.id == "q3-demota").unwrap();
        assert_eq!(demo.edition, "demo");
        assert!(demo.base_product.is_none());
        assert!(products.iter().all(|product| product.unresolved_reason.is_none()));
    }

    #[test]
    fn native_timing_matches_families() {
        let provider = ProviderReference {
            provider: provider_id("q1:source/classic/id1"),
            content: create_content_id(&ContentIdentity {
                family: GameFamily::Q1,
                edition: "classic".to_string(),
                package: "id1".to_string(),
                revision: "installed".to_string(),
            })
            .unwrap(),
        };
        let q1 = native_provider_timing(&provider, GameFamily::Q1, false);
        assert_eq!(q1.numeric.id, "q1:binary32");
        assert!(matches!(
            q1.clock,
            ClockProfile::Q1Netquake {
                minimum_frame_seconds: 0.001,
                maximum_frame_seconds: 0.1,
                fixed_frame_seconds: None
            }
        ));
        let classic = native_provider_timing(&provider, GameFamily::Q2, false);
        assert_eq!(classic.numeric.id, "q2:binary32");
        assert_eq!(classic.clock, ClockProfile::Q2Classic);
        let rerelease = native_provider_timing(&provider, GameFamily::Q2, true);
        assert_eq!(
            rerelease.clock,
            ClockProfile::Q2Rerelease {
                frame_milliseconds: 25.0
            }
        );
        let q3 = native_provider_timing(&provider, GameFamily::Q3, false);
        assert_eq!(q3.numeric.id, "q3:binary32");
        assert_eq!(
            q3.clock,
            ClockProfile::Q3 {
                server_frame_milliseconds: 50.0,
                fixed_movement_milliseconds: None
            }
        );
    }

    #[test]
    fn weapon_behavior_documents_validate() {
        let document = read_weapon_behavior_document(
            br#"{"version": 1, "artifactPath": "vm/qagame.qvm", "behaviors": [{"id": "a"}]}"#,
        )
        .unwrap();
        assert_eq!(document.version, 1);
        assert_eq!(document.artifact_path.as_deref(), Some("vm/qagame.qvm"));
        assert_eq!(document.behaviors.len(), 1);
        assert_eq!(weapon_behavior_entry_id(&document.behaviors[0]).unwrap(), "a");
        assert!(read_weapon_behavior_document(br#"{"version": 2, "behaviors": []}"#).is_err());
        assert!(read_weapon_behavior_document(br#"{"version": 1}"#).is_err());
        assert!(read_weapon_behavior_document(br#"[]"#).is_err());
        assert!(read_weapon_behavior_document(b"\xff\xfe").is_err());
        let bad_artifact =
            read_weapon_behavior_document(br#"{"version": 1, "artifactPath": 7, "behaviors": []}"#).unwrap_err();
        assert_eq!(bad_artifact.to_string(), "Invalid weapon behavior artifact path");
        assert!(weapon_behavior_entry_id(&parse_save_json(r#"{"id": 7}"#).unwrap()).is_err());
    }

    #[test]
    fn authored_starts_parse_episodes_and_maps() {
        let bytes = br#"{
            "episodes": [{"id": "baseq2", "command": "newgame", "name": "Base", "activity": "act", "needsSkillSelect": true}],
            "maps": [
                {"episode": "baseq2", "bsp": "intro+base1", "title": "Base", "start_items": "1", "sp": true, "coop": false, "ctf": false},
                {"episode": "baseq2", "bsp": "*secret", "title": "Secret", "sp": false, "coop": false, "ctf": true},
                {"episode": "other", "bsp": "x", "sp": true}
            ]
        }"#;
        let parsed = parse_authored_starts(bytes, "baseq2").unwrap();
        assert_eq!(parsed.episode.as_ref().unwrap().command, "newgame");
        assert!(parsed.episode.as_ref().unwrap().needs_skill_select);
        assert_eq!(parsed.starts.len(), 1);
        assert_eq!(parsed.starts[0].path, "maps/base1.bsp");
        assert_eq!(parsed.starts[0].bsp, "intro+base1");
        assert!(parsed.starts[0].singleplayer);
        let ctf = parse_authored_starts(bytes, "ctf").unwrap();
        assert_eq!(ctf.starts.len(), 1);
        assert_eq!(ctf.starts[0].path, "maps/secret.bsp");
        assert_eq!(ctf.starts[0].episode, "baseq2");
        assert!(parse_authored_starts(
            br#"{"episodes": [], "maps": [{"episode": "baseq2", "bsp": "a;b", "sp": true}]}"#,
            "baseq2"
        )
        .is_err());
        assert!(parse_authored_starts(br#"[]"#, "baseq2").is_err());
    }

    #[test]
    fn source_programs_resolve_through_mods() {
        let base = installed_product("q1-classic-id1", GameFamily::Q1, "classic", "id1", None);
        let mut mod_product = installed_product("mod", GameFamily::Q1, "classic", "mymod", Some("q1-classic-id1"));
        mod_product.expectation.id = "q1-classic-mymod".to_string();
        mod_product.expectation.content_directory = "q1/mymod".to_string();
        let catalog = test_catalog(vec![base, mod_product]);
        let resolved = source_program_product(&catalog, "q1-classic-mymod").unwrap();
        assert_eq!(resolved.expectation.id, "q1-classic-id1");
        let stock = source_program_product(&catalog, "q1-classic-id1").unwrap();
        assert_eq!(stock.expectation.id, "q1-classic-id1");
        let implementation = source_program_implementation(&resolved.expectation);
        assert_eq!(provider_text(&implementation), "q1:source/classic/id1");
    }

    #[test]
    fn source_program_cycles_fail() {
        let mut first = installed_product("a", GameFamily::Q1, "classic", "a", Some("b-id"));
        first.expectation.id = "a-id".to_string();
        let mut second = installed_product("b", GameFamily::Q1, "classic", "b", Some("a-id"));
        second.expectation.id = "b-id".to_string();
        let catalog = test_catalog(vec![first, second]);
        assert!(source_program_product(&catalog, "a-id").is_err());
    }

    #[test]
    fn selected_source_programs_map_implementations() {
        let entities = ProviderReference {
            provider: provider_id("q2:source/classic/baseq2"),
            content: create_content_id(&ContentIdentity {
                family: GameFamily::Q2,
                edition: "classic".to_string(),
                package: "baseq2".to_string(),
                revision: "installed".to_string(),
            })
            .unwrap(),
        };
        let empty: Vec<ExecutionModule<ResourceRequest>> = Vec::new();
        assert_eq!(
            selected_source_program(&empty, &entities).unwrap(),
            Some("baseq2".to_string())
        );
        let replacement: ExecutionModule<ResourceRequest> = ExecutionModule::Builtin {
            owner: entities.clone(),
            implementation: provider_id("q2:source/classic/xatrix"),
            role: ModuleRole::ServerGame,
            api: SourceModuleApi::Q2ClassicGame,
        };
        assert_eq!(
            selected_source_program(&[replacement], &entities).unwrap(),
            Some("xatrix".to_string())
        );
        let unknown: ExecutionModule<ResourceRequest> = ExecutionModule::Builtin {
            owner: entities.clone(),
            implementation: provider_id("q2:source/classic/nope"),
            role: ModuleRole::ServerGame,
            api: SourceModuleApi::Q2ClassicGame,
        };
        assert!(selected_source_program(&[unknown], &entities).is_err());
    }

    #[test]
    fn monster_reexports_cover_roster() {
        assert!(!monster_sources().is_empty());
        assert!(!campaign_monster_slots(GameFamily::Q1).is_empty());
        assert!(!campaign_monster_slots(GameFamily::Q2).is_empty());
        assert!(campaign_monster_slots(GameFamily::Q3).is_empty());
        let source = monster_sources()
            .into_iter()
            .find(|source| {
                source.family == MonsterFamily::Q1
                    && source.program == MonsterProgram::Id1
                    && source.edition == SourceEdition::Classic
            })
            .unwrap();
        let target = ProviderReference {
            provider: source.provider.clone(),
            content: create_content_id(&ContentIdentity {
                family: GameFamily::Q1,
                edition: "classic".to_string(),
                package: "id1".to_string(),
                revision: "installed".to_string(),
            })
            .unwrap(),
        };
        let roster = default_monster_roster(MonsterFamily::Q1, &target, &HashMap::new()).unwrap();
        assert!(matches!(roster, EnemySelection::Replace { .. }));
    }

    fn monster_test_setup() -> (InstalledCatalog, MonsterDefinitionReference) {
        let source = monster_sources()
            .into_iter()
            .find(|source| {
                source.family == MonsterFamily::Q1
                    && source.program == MonsterProgram::Id1
                    && source.edition == SourceEdition::Classic
            })
            .unwrap();
        let classname = source.creatures.keys().next().unwrap().clone();
        let product = installed_product("q1-classic-id1", GameFamily::Q1, "classic", "id1", None);
        let definition = MonsterDefinitionReference {
            source: ProviderReference {
                provider: source.provider.clone(),
                content: product.id.clone(),
            },
            classname,
        };
        (test_catalog(vec![product]), definition)
    }

    #[test]
    fn monster_selections_validate_and_resolve() {
        let (catalog, definition) = monster_test_setup();
        assert!(selected_monster_definitions(&EnemySelection::MapDefined).is_empty());
        let enemies = EnemySelection::Replace {
            default: MonsterSelectionTarget::Defined(definition.clone()),
            by_classname: HashMap::from([("other".to_string(), MonsterSelectionTarget::MapDefined)]),
        };
        assert_eq!(selected_monster_definitions(&enemies), vec![definition.clone()]);
        validate_monsters(&enemies, &catalog).unwrap();
        let timings = selected_monster_timing(&enemies);
        assert_eq!(timings.len(), 1);
        assert_eq!(timings[0].provider, definition.source.provider);
        let resources = monster_resources(&enemies).unwrap();
        assert!(!resources.is_empty());
        assert!(resources
            .iter()
            .all(|request| request.content == definition.source.content));
    }

    #[test]
    fn monster_mismatches_fail() {
        let (_, definition) = monster_test_setup();
        let wrong_content = MonsterDefinitionReference {
            source: ProviderReference {
                provider: definition.source.provider.clone(),
                content: create_content_id(&ContentIdentity {
                    family: GameFamily::Q2,
                    edition: "classic".to_string(),
                    package: "baseq2".to_string(),
                    revision: "installed".to_string(),
                })
                .unwrap(),
            },
            classname: definition.classname.clone(),
        };
        let mut product = installed_product("q2-classic-baseq2", GameFamily::Q2, "classic", "baseq2", None);
        product.id = wrong_content.source.content.clone();
        let catalog = test_catalog(vec![product]);
        let enemies = EnemySelection::Replace {
            default: MonsterSelectionTarget::Defined(wrong_content),
            by_classname: HashMap::new(),
        };
        assert!(validate_monsters(&enemies, &catalog).is_err());
        let empty_key = EnemySelection::Replace {
            default: MonsterSelectionTarget::MapDefined,
            by_classname: HashMap::from([("".to_string(), MonsterSelectionTarget::MapDefined)]),
        };
        let error = validate_monsters(&empty_key, &catalog).unwrap_err();
        assert_eq!(error.to_string(), "An authored monster classname cannot be empty");
    }

    #[test]
    fn grapple_styles_prefer_installed_editions() {
        let mut classic = installed_product("q1-classic-ctf", GameFamily::Q1, "classic", "ctf", None);
        let mut rerelease = installed_product("q1-rerelease-ctf", GameFamily::Q1, "rerelease", "ctf", None);
        rerelease.availability = ProductAvailability::Missing {
            requirements: vec!["q1/rerelease/ctf/pak0.pak".to_string()],
        };
        let lmctf = installed_product("q2-classic-lmctf", GameFamily::Q2, "classic", "lmctf", None);
        let catalog = test_catalog(vec![rerelease.clone(), classic.clone(), lmctf]);
        let styles = grapple_styles(&catalog, &classic);
        assert_eq!(styles.len(), 2);
        assert_eq!(styles[0].id, "q1-threewave");
        assert_eq!(styles[0].title, "Threewave CTF (Quake 1)");
        assert!(styles[0].unavailable.is_none());
        assert_eq!(styles[1].id, "q2-lmctf");
        classic.availability = ProductAvailability::Missing {
            requirements: vec!["q1/ctf/pak0.pak".to_string()],
        };
        let catalog = test_catalog(vec![classic.clone(), rerelease]);
        let styles = grapple_styles(&catalog, &classic);
        assert_eq!(styles.len(), 1);
        assert_eq!(
            styles[0].unavailable.as_deref(),
            Some("Requires q1-classic-ctf game files")
        );
        let unresolved = installed_product("q2-classic-ctf", GameFamily::Q2, "classic", "ctf", None);
        let mut catalog_products = vec![unresolved];
        catalog_products[0].availability = ProductAvailability::Unresolved {
            reason: "nope".to_string(),
        };
        let preferred = catalog_products[0].clone();
        let catalog = test_catalog(catalog_products);
        let styles = grapple_styles(&catalog, &preferred);
        assert_eq!(styles[0].unavailable.as_deref(), Some("nope"));
    }

    #[test]
    fn offhand_grenades_prefer_editions() {
        let classic = installed_product("q2-classic-baseq2", GameFamily::Q2, "classic", "baseq2", None);
        let rerelease = installed_product("q2-rerelease-baseq2", GameFamily::Q2, "rerelease", "baseq2", None);
        let catalog = test_catalog(vec![classic, rerelease.clone()]);
        assert_eq!(offhand_grenade_source(&catalog, &rerelease).unwrap().id, rerelease.id);
        let mut missing = installed_product("q2-classic-baseq2", GameFamily::Q2, "classic", "baseq2", None);
        missing.availability = ProductAvailability::Missing {
            requirements: Vec::new(),
        };
        let preferred = missing.clone();
        let catalog = test_catalog(vec![missing]);
        assert!(offhand_grenade_source(&catalog, &preferred).is_none());
    }

    #[test]
    fn native_equipment_follows_map_and_match() {
        let ctf = installed_product("q2-classic-ctf", GameFamily::Q2, "classic", "ctf", None);
        let base = installed_product("q2-classic-baseq2", GameFamily::Q2, "classic", "baseq2", None);
        let catalog = test_catalog(vec![ctf.clone(), base.clone()]);
        let map = ProviderReference {
            provider: provider_id("q2:map"),
            content: ctf.id.clone(),
        };
        let rules = ProviderReference {
            provider: provider_id("q2:ctf"),
            content: base.id.clone(),
        };
        let native = native_equipment(&catalog, &map, &rules).unwrap();
        assert_eq!(native.grapple, GrappleSelection::Disabled);
        let other = ProviderReference {
            provider: provider_id("q2:dm"),
            content: base.id.clone(),
        };
        let native = native_equipment(&catalog, &map, &other).unwrap();
        assert!(matches!(
            native.grapple,
            GrappleSelection::Enabled {
                mechanic: GrappleMechanicDetail::Q2Ctf { .. },
                ..
            }
        ));
        let q1ctf = installed_product("q1-classic-ctf", GameFamily::Q1, "classic", "ctf", None);
        let catalog = test_catalog(vec![q1ctf.clone(), base.clone()]);
        let map = ProviderReference {
            provider: provider_id("q1:map"),
            content: q1ctf.id.clone(),
        };
        let native = native_equipment(&catalog, &map, &other).unwrap();
        assert!(matches!(
            native.grapple,
            GrappleSelection::Enabled {
                mechanic: GrappleMechanicDetail::Q1Threewave { .. },
                ..
            }
        ));
    }

    #[test]
    fn equipment_validates_sources_and_allowances() {
        let providers = equipment_providers();
        let ctf = installed_product("q1-classic-ctf", GameFamily::Q1, "classic", "ctf", None);
        let base = installed_product("q2-classic-baseq2", GameFamily::Q2, "classic", "baseq2", None);
        let catalog = test_catalog(vec![ctf.clone(), base.clone()]);
        let grapple = EquipmentSelection {
            grapple: GrappleSelection::Enabled {
                source: ProviderReference {
                    provider: providers.threewave.clone(),
                    content: ctf.id.clone(),
                },
                binding: GrappleBinding::Slot,
                mechanic: GrappleMechanicDetail::Q1Threewave {
                    edition: SourceEdition::Classic,
                },
            },
            hand_grenades: HandGrenadeSelection::Enabled {
                source: ProviderReference {
                    provider: providers.hand_grenades.clone(),
                    content: base.id.clone(),
                },
                edition: SourceEdition::Classic,
                initial_ammo: 5.0,
                capacity: 10.0,
            },
        };
        validate_equipment(&grapple, &catalog).unwrap();
        assert_eq!(equipment_provider_refs(&grapple).len(), 2);
        assert!(equipment_provider_refs(&disabled_equipment()).is_empty());
        let wrong_edition = EquipmentSelection {
            grapple: GrappleSelection::Enabled {
                source: ProviderReference {
                    provider: providers.threewave.clone(),
                    content: ctf.id.clone(),
                },
                binding: GrappleBinding::Slot,
                mechanic: GrappleMechanicDetail::Q1Threewave {
                    edition: SourceEdition::Rerelease,
                },
            },
            hand_grenades: HandGrenadeSelection::Disabled,
        };
        assert!(validate_equipment(&wrong_edition, &catalog).is_err());
        let over_capacity = EquipmentSelection {
            grapple: GrappleSelection::Disabled,
            hand_grenades: HandGrenadeSelection::Enabled {
                source: ProviderReference {
                    provider: providers.hand_grenades.clone(),
                    content: base.id.clone(),
                },
                edition: SourceEdition::Classic,
                initial_ammo: 11.0,
                capacity: 10.0,
            },
        };
        let error = validate_equipment(&over_capacity, &catalog).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Equipment hand grenade allowance must be whole ammunition within capacity"
        );
    }

    fn qvm_test_profile(module: ModuleIdentity) -> QvmGrappleDefinition {
        QvmGrappleDefinition {
            id: "hook".to_string(),
            title: "Hook".to_string(),
            module,
            abi_profile: QvmAbiProfile::Modern,
            entity_stride: 8,
            client_stride: 8,
            fields: QvmGrappleFields {
                inuse: 0,
                client: 1,
                parent: 2,
                target: 3,
                mover: None,
                hook: 4,
                health: 5,
                takedamage: 6,
                event_time: 7,
                free_after_event: 8,
            },
            globals: QvmGrappleGlobals {
                time: 0,
                frame: 1,
                movement: 2,
                forward: 3,
                ground_plane: 4,
            },
            callbacks: QvmGrappleCallbacks {
                allocate: 0,
                free: 1,
                fire: 2,
                release: 3,
                force_release: 4,
                missile: 5,
                follow: None,
                think: 6,
                pull: 7,
                move_mover_hooks: None,
                damage: 8,
                same_team: 9,
                player_move: 10,
            },
            fire_arguments: Vec::new(),
            movement: QvmGrappleMovement {
                byte_length: 0,
                words: Vec::new(),
            },
            initial_cvars: HashMap::new(),
            event_lifetime_milliseconds: 0.0,
            grapple_damage_method: 0.0,
            presentation: QvmGrapplePresentation {
                projectile_model: "models/hook/tris.md3".to_string(),
                view_model: "models/v_hook/tris.md3".to_string(),
                weapon_index: 1.0,
                view_anchor: QvmGrappleViewAnchor {
                    path: "models/anchor/tris.md3".to_string(),
                    tag: "tag".to_string(),
                    offset: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                    fov_offset: QvmGrappleFovOffset { above: 0.0, scale: 1.0 },
                },
                view_attachments: vec![QvmGrappleViewAttachment {
                    path: "models/attach/tris.md3".to_string(),
                    tag: "tag".to_string(),
                }],
                cable: QvmGrappleCable::Model {
                    flight: "models/cable/fly.md3".to_string(),
                    pull: "models/cable/pull.md3".to_string(),
                    hold: "models/cable/hold.md3".to_string(),
                    segment_length: 1.0,
                },
                fire_sound: Some("sound/hook/fire.wav".to_string()),
                attach_sound: None,
                release_sound: None,
                pull_sound: None,
                hang_sound: None,
            },
            pulling_flag: 0.0,
        }
    }

    #[test]
    fn equipment_qvm_hooks_validate_and_list_resources() {
        let base = installed_product("q3-baseq3", GameFamily::Q3, "classic", "baseq3", None);
        let catalog = test_catalog(vec![base.clone()]);
        let provider = provider_id("q3:grapple/hook");
        let profile = qvm_test_profile(ModuleIdentity {
            id: provider.clone(),
            artifact_path: "vm/hook.qvm".to_string(),
            digest: create_content_digest(&"cd".repeat(32)).unwrap(),
            revision: "1".to_string(),
        });
        let equipment = EquipmentSelection {
            grapple: GrappleSelection::Enabled {
                source: ProviderReference {
                    provider: provider.clone(),
                    content: base.id.clone(),
                },
                binding: GrappleBinding::Slot,
                mechanic: GrappleMechanicDetail::Q3Qvm {
                    profile: profile.clone(),
                },
            },
            hand_grenades: HandGrenadeSelection::Disabled,
        };
        validate_equipment(&equipment, &catalog).unwrap();
        let timings = equipment_timing(&equipment);
        assert_eq!(timings.len(), 1);
        assert_eq!(timings[0].numeric, Q3_BINARY32_PROFILE);
        let resources = equipment_resources(&equipment);
        let paths: Vec<&str> = resources.iter().map(|request| request.path.as_str()).collect();
        assert!(paths.contains(&"vm/hook.qvm"));
        assert!(paths.contains(&"models/hook/tris.md3"));
        assert!(paths.contains(&"models/cable/hold.md3"));
        assert!(paths.contains(&"sound/hook/fire.wav"));
        let mismatched = EquipmentSelection {
            grapple: GrappleSelection::Enabled {
                source: ProviderReference {
                    provider: provider_id("q3:grapple/other"),
                    content: base.id.clone(),
                },
                binding: GrappleBinding::Slot,
                mechanic: GrappleMechanicDetail::Q3Qvm { profile },
            },
            hand_grenades: HandGrenadeSelection::Disabled,
        };
        assert!(validate_equipment(&mismatched, &catalog).is_err());
    }

    #[test]
    fn equipment_resources_cover_source_hooks() {
        let providers = equipment_providers();
        let ctf = installed_product("q1-classic-ctf", GameFamily::Q1, "classic", "ctf", None);
        let classic = EquipmentSelection {
            grapple: GrappleSelection::Enabled {
                source: ProviderReference {
                    provider: providers.threewave.clone(),
                    content: ctf.id.clone(),
                },
                binding: GrappleBinding::Slot,
                mechanic: GrappleMechanicDetail::Q1Threewave {
                    edition: SourceEdition::Classic,
                },
            },
            hand_grenades: HandGrenadeSelection::Disabled,
        };
        assert_eq!(equipment_resources(&classic).len(), 9);
        let rerelease = EquipmentSelection {
            grapple: GrappleSelection::Enabled {
                source: ProviderReference {
                    provider: providers.threewave.clone(),
                    content: ctf.id.clone(),
                },
                binding: GrappleBinding::Offhand,
                mechanic: GrappleMechanicDetail::Q1Threewave {
                    edition: SourceEdition::Rerelease,
                },
            },
            hand_grenades: HandGrenadeSelection::Disabled,
        };
        assert_eq!(equipment_resources(&rerelease).len(), 5);
        let timings = equipment_timing(&classic);
        assert_eq!(timings[0].numeric, Q1_DONOR_PROFILE);
        let q2base = installed_product("q2-classic-baseq2", GameFamily::Q2, "classic", "baseq2", None);
        let grenades = EquipmentSelection {
            grapple: GrappleSelection::Disabled,
            hand_grenades: HandGrenadeSelection::Enabled {
                source: ProviderReference {
                    provider: providers.hand_grenades.clone(),
                    content: q2base.id.clone(),
                },
                edition: SourceEdition::Rerelease,
                initial_ammo: 0.0,
                capacity: 5.0,
            },
        };
        let resources = equipment_resources(&grenades);
        assert!(resources.iter().any(|request| request.path.contains("grenade3")));
        assert_eq!(equipment_timing(&grenades)[0].numeric, Q2_DONOR_PROFILE);
    }

    fn quaddicted_sha(seed: &str) -> String {
        format!("{seed}{}", "ab".repeat(32))[..64].to_string()
    }

    fn quaddicted_row(sha: &str, tags: &[&str], install: &str) -> String {
        let tags = tags
            .iter()
            .map(|tag| format!(r#""{tag}""#))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            r#"{{"sha256": "{sha}", "bytes": 4242, "tags": [{tags}],
            "urls": ["https://www.quaddicted.com/files/by-sha256/{}/{sha}/test.zip"], "install": {install}}}"#,
            &sha[..2]
        )
    }

    #[test]
    fn addon_catalogs_parse_and_filter() {
        let sha = quaddicted_sha("00");
        let row = quaddicted_row(
            &sha,
            &[
                "game=quake",
                "game_mode=singleplayer",
                "filename=test.zip",
                "title=Test Map",
                "startmap=e1m1",
                "startmap=../escape",
                "commandline=-game test",
            ],
            r#"{"extract": "{base}/test/"}"#,
        );
        let skipped = quaddicted_row(
            &quaddicted_sha("11"),
            &["game=quake", "game_mode=deathmatch", "filename=skip.zip"],
            r#"{"extract": "x"}"#,
        );
        let value = parse_save_json(&format!("[{row},{skipped}]")).unwrap();
        let catalog = parse_addon_catalog(&value).unwrap();
        assert_eq!(catalog.len(), 1);
        let package = &catalog[0];
        assert_eq!(package.title, "Test Map");
        assert_eq!(package.group, "test");
        assert_eq!(package.game_directory, "test");
        assert_eq!(package.starts, vec!["e1m1".to_string()]);
        assert_eq!(
            package.mappings,
            vec![AddonMapping {
                from: String::new(),
                to: Some("test/".to_string())
            }]
        );
        assert!(package.unavailable.is_none());
        assert_eq!(package.digest.as_str(), format!("sha256:{sha}"));
    }

    #[test]
    fn addon_catalogs_report_identity_and_install_problems() {
        let bad_identity = quaddicted_row(
            &"zz".repeat(32),
            &["game=quake", "game_mode=singleplayer", "filename=x.zip"],
            r#"{"extract": "x"}"#,
        );
        let error = parse_addon_catalog(&parse_save_json(&format!("[{bad_identity}]")).unwrap()).unwrap_err();
        assert_eq!(error.to_string(), "Invalid Quaddicted package identity: x.zip");
        let sha = quaddicted_sha("22");
        let unsupported = quaddicted_row(
            &sha,
            &[
                "game=quake",
                "game_mode=singleplayer",
                "filename=u.zip",
                "commandline=-rogue",
            ],
            r#"{"extract": "x"}"#,
        );
        let catalog = parse_addon_catalog(&parse_save_json(&format!("[{unsupported}]")).unwrap()).unwrap();
        assert_eq!(
            catalog[0].unavailable.as_deref(),
            Some("Requires a source gameplay option not yet supported by this installer")
        );
        let no_install = quaddicted_row(
            &quaddicted_sha("33"),
            &["game=quake", "game_mode=singleplayer", "filename=n.zip"],
            r#"{"extractmapping": {"a": 7}}"#,
        );
        let catalog = parse_addon_catalog(&parse_save_json(&format!("[{no_install}]")).unwrap()).unwrap();
        assert_eq!(
            catalog[0].unavailable.as_deref(),
            Some("Unsupported extraction mapping")
        );
        assert!(parse_addon_catalog(&parse_save_json(r#"{"not": "array"}"#).unwrap()).is_err());
        let bad_url = format!(
            r#"[{{"sha256": "{sha}", "bytes": 1, "tags": ["game=quake", "game_mode=singleplayer", "filename=u.zip"], "urls": ["not a url"], "install": {{"extract": "x"}}}}]"#
        );
        assert!(parse_addon_catalog(&parse_save_json(&bad_url).unwrap()).is_err());
        let foreign_url = format!(
            r#"[{{"sha256": "{sha}", "bytes": 1, "tags": ["game=quake", "game_mode=singleplayer", "filename=u.zip"], "urls": ["https://example.com/x.zip"], "install": {{"extract": "x"}}}}]"#
        );
        assert!(parse_addon_catalog(&parse_save_json(&foreign_url).unwrap())
            .unwrap()
            .is_empty());
    }

    fn addon_package(title: &str, filename: &str, sha: &str, tags: &[&str]) -> AddonPackage {
        AddonPackage {
            digest: create_content_digest(sha).unwrap(),
            sha256: sha.to_string(),
            title: title.to_string(),
            filename: filename.to_string(),
            group: title.to_string(),
            bytes: 1,
            url: format!("https://www.quaddicted.com/files/by-sha256/{}/{sha}/x.zip", &sha[..2]),
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
            starts: Vec::new(),
            game_directory: "id1".to_string(),
            mappings: Vec::new(),
            unavailable: None,
        }
    }

    #[test]
    fn addon_install_paths_follow_mappings() {
        let mut package = addon_package("t", "t.zip", &quaddicted_sha("44"), &[]);
        package.mappings = vec![
            AddonMapping {
                from: String::new(),
                to: Some("quoth/".to_string()),
            },
            AddonMapping {
                from: "maps/".to_string(),
                to: Some("maps/".to_string()),
            },
            AddonMapping {
                from: "skip.txt".to_string(),
                to: None,
            },
        ];
        assert_eq!(
            addon_install_path(&package, "maps/a.bsp").unwrap().as_deref(),
            Some("id1/maps/a.bsp")
        );
        assert_eq!(
            addon_install_path(&package, "readme.txt").unwrap().as_deref(),
            Some("quoth/readme.txt")
        );
        assert_eq!(addon_install_path(&package, "skip.txt").unwrap(), None);
        package.mappings.clear();
        assert_eq!(addon_install_path(&package, "maps/a.bsp").unwrap(), None);
        assert!(addon_install_path(&package, "../escape").is_err());
    }

    #[test]
    fn addon_dependencies_resolve_in_order() {
        let lib = addon_package("lib", "lib.zip", &quaddicted_sha("55"), &["provides=lib=2.0"]);
        let mut app = addon_package("app", "app.zip", &quaddicted_sha("66"), &["depends='lib>=1.0'"]);
        let catalog = vec![lib.clone(), app.clone()];
        let resolved = resolve_addon_packages(&catalog, &app).unwrap();
        assert_eq!(
            resolved
                .iter()
                .map(|package| package.title.as_str())
                .collect::<Vec<_>>(),
            vec!["lib", "app"]
        );
        app.tags = vec!["depends='lib>=3.0'".to_string()];
        assert!(resolve_addon_packages(&catalog, &app).is_err());
        app.tags = vec!["depends='lib'".to_string()];
        assert!(resolve_addon_packages(&catalog, &app).is_err());
        app.tags = vec!["dependency=lib".to_string()];
        let resolved = resolve_addon_packages(&catalog, &app).unwrap();
        assert_eq!(resolved.len(), 2);
        app.tags = vec!["dependency=missing".to_string()];
        assert!(resolve_addon_packages(&catalog, &app).is_err());
    }

    #[test]
    fn addon_dependency_cycles_fail() {
        let first = addon_package("a", "a.zip", &quaddicted_sha("88"), &["dependency=b.zip"]);
        let second = addon_package("b", "b.zip", &quaddicted_sha("99"), &["dependency=a"]);
        let catalog = vec![first.clone(), second];
        assert!(resolve_addon_packages(&catalog, &first).is_err());
        let mut unavailable = addon_package("u", "u.zip", &quaddicted_sha("aa"), &[]);
        unavailable.unavailable = Some("broken".to_string());
        let catalog = vec![unavailable.clone()];
        let error = resolve_addon_packages(&catalog, &unavailable).unwrap_err();
        assert_eq!(error.to_string(), "u: broken");
    }

    fn test_archive(path: &str, format: ArchiveFormat) -> CatalogArchive {
        CatalogArchive {
            path: path.to_string(),
            format,
            entries: Vec::new(),
        }
    }

    #[test]
    fn archive_ordering_matches_source_engines() {
        let q1 = expectation("q1-classic-id1", GameFamily::Q1, "classic", "id1", None);
        let archives = vec![
            test_archive("/c/pak2.pak", ArchiveFormat::Pak),
            test_archive("/c/pak0.pak", ArchiveFormat::Pak),
            test_archive("/c/extra.pk3", ArchiveFormat::Pk3),
            test_archive("/c/notes.zip", ArchiveFormat::Zip),
        ];
        let ordered = order_game_archives(&q1, &archives);
        assert_eq!(
            ordered
                .iter()
                .map(|archive| archive_basename(&archive.path))
                .collect::<Vec<_>>(),
            vec!["extra.pk3".to_string(), "pak0.pak".to_string()]
        );
        let q3 = expectation("q3-baseq3", GameFamily::Q3, "classic", "baseq3", None);
        let archives = vec![
            test_archive("/c/a.pk3", ArchiveFormat::Pk3),
            test_archive("/c/B.pk3", ArchiveFormat::Pk3),
            test_archive("/c/c.pak", ArchiveFormat::Pak),
        ];
        let ordered = order_game_archives(&q3, &archives);
        assert_eq!(
            ordered
                .iter()
                .map(|archive| archive_basename(&archive.path))
                .collect::<Vec<_>>(),
            vec!["B.pk3".to_string(), "a.pk3".to_string()]
        );
    }

    #[test]
    fn q2_archive_ordering_compares_numeric_prefixes() {
        let q2 = expectation("q2-classic-baseq2", GameFamily::Q2, "classic", "baseq2", None);
        let archives = vec![
            test_archive("/c/pak0.pak", ArchiveFormat::Pak),
            test_archive("/c/pak10.pak", ArchiveFormat::Pak),
            test_archive("/c/pak2.pak", ArchiveFormat::Pak),
            test_archive("/c/mod.pkz", ArchiveFormat::Zip),
            test_archive("/c/readme.zip", ArchiveFormat::Zip),
        ];
        let ordered = order_game_archives(&q2, &archives);
        assert_eq!(
            ordered
                .iter()
                .map(|archive| archive_basename(&archive.path))
                .collect::<Vec<_>>(),
            vec![
                "mod.pkz".to_string(),
                "pak10.pak".to_string(),
                "pak2.pak".to_string(),
                "pak0.pak".to_string()
            ]
        );
        let archives = vec![
            test_archive("/c/pak0.pak", ArchiveFormat::Pak),
            test_archive("/c/pak99999999999999999999999.pak", ArchiveFormat::Pak),
            test_archive("/c/pak-2.pak", ArchiveFormat::Pak),
        ];
        let ordered = order_game_archives(&q2, &archives);
        // The overflowing prefix clamps to the maximum; the negative wraps below it.
        assert_eq!(archive_basename(&ordered[0].path), "pak99999999999999999999999.pak");
        assert_eq!(archive_basename(&ordered[1].path), "pak-2.pak");
        assert_eq!(archive_basename(&ordered[2].path), "pak0.pak");
        let archives = vec![
            test_archive("/c/pak0.pak", ArchiveFormat::Pak),
            test_archive("/c/pak0a.pak", ArchiveFormat::Pak),
        ];
        let ordered = order_game_archives(&q2, &archives);
        assert_eq!(archive_basename(&ordered[0].path), "pak0a.pak");
    }

    #[test]
    fn remote_selections_normalize_directories() {
        let selection = remote_content_selection(RemoteContentBase::Q1Quakeworld, "ID1").unwrap();
        assert_eq!(selection.directory, "qw");
        let selection = remote_content_selection(RemoteContentBase::Q2ClassicBaseq2, "").unwrap();
        assert_eq!(selection.directory, "baseq2");
        let selection = remote_content_selection(RemoteContentBase::Q3Baseq3, "").unwrap();
        assert_eq!(selection.directory, "baseq3");
        let selection = remote_content_selection(RemoteContentBase::Q2ClassicBaseq2, "coop").unwrap();
        assert_eq!(selection.directory, "coop");
        assert!(remote_content_selection(RemoteContentBase::Q2ClassicBaseq2, "..").is_err());
        assert!(remote_content_selection(RemoteContentBase::Q2ClassicBaseq2, ".").is_err());
        assert!(remote_content_selection(RemoteContentBase::Q2ClassicBaseq2, "a/b").is_err());
        let known = remote_content_selection(RemoteContentBase::Q2ClassicBaseq2, "xatrix").unwrap();
        assert_eq!(remote_content_product(&known, None).unwrap(), "q2-classic-xatrix");
        let synthetic = remote_content_selection(RemoteContentBase::Q2ClassicBaseq2, "coopmod").unwrap();
        assert_eq!(
            remote_content_product(&synthetic, None).unwrap(),
            "q2-classic-baseq2-mod-coopmod"
        );
        let missing_base = remote_content_selection(RemoteContentBase::Q3Baseq3, "arena").unwrap();
        assert!(remote_content_product(&missing_base, Some(&[])).is_err());
    }

    #[test]
    fn remote_foreign_products_fail_inheritance() {
        let stock = vec![
            expectation("q2-classic-baseq2", GameFamily::Q2, "classic", "baseq2", None),
            ProductExpectation {
                content_directory: "q2/odd".to_string(),
                base_product: Some("unrelated".to_string()),
                ..expectation("other", GameFamily::Q2, "classic", "odd", Some("unrelated"))
            },
        ];
        let selection = remote_content_selection(RemoteContentBase::Q2ClassicBaseq2, "odd").unwrap();
        assert!(remote_content_product(&selection, Some(&stock)).is_err());
    }

    #[test]
    fn catalog_identities_reject_duplicates_and_unknowns() {
        let product = installed_product("q1-classic-id1", GameFamily::Q1, "classic", "id1", None);
        let duplicate = product.clone();
        assert!(InstalledCatalog::new("/c".to_string(), vec![product, duplicate], Vec::new(), 0, None).is_err());
        let product = installed_product("q1-classic-id1", GameFamily::Q1, "classic", "id1", None);
        let catalog = test_catalog(vec![product]);
        assert!(catalog.product("nope").is_err());
        assert!(catalog.product("q1:classic:id1:installed").is_ok());
        assert!(catalog.product("q1-classic-id1").is_ok());
        let mut missing = installed_product("m", GameFamily::Q1, "classic", "m", None);
        missing.expectation.id = "m-id".to_string();
        missing.availability = ProductAvailability::Missing {
            requirements: vec!["q1/m/pak0.pak".to_string()],
        };
        let catalog = test_catalog(vec![missing]);
        let error = catalog.require("m-id").unwrap_err();
        assert_eq!(error.to_string(), "Content m-id requires: q1/m/pak0.pak");
    }

    #[test]
    fn map_winners_follow_archives_and_bases() {
        let mut base = installed_product("q1-classic-id1", GameFamily::Q1, "classic", "id1", None);
        base.loose_root = Some("/corpus/q1/id1".to_string());
        base.archives = vec![
            CatalogArchive {
                path: "/corpus/q1/id1/pak0.pak".to_string(),
                format: ArchiveFormat::Pak,
                entries: Vec::new(),
            },
            CatalogArchive {
                path: "/corpus/q1/id1/extra.pk3".to_string(),
                format: ArchiveFormat::Pk3,
                entries: Vec::new(),
            },
        ];
        base.maps = vec![
            ContentMap {
                path: "maps/shared.bsp".to_string(),
                source: "/corpus/q1/id1/pak0.pak".to_string(),
                member_index: Some(0),
            },
            ContentMap {
                path: "maps/dup.bsp".to_string(),
                source: "/corpus/q1/id1/extra.pk3".to_string(),
                member_index: Some(0),
            },
            ContentMap {
                path: "maps/dup.bsp".to_string(),
                source: "/corpus/q1/id1/extra.pk3".to_string(),
                member_index: Some(1),
            },
            ContentMap {
                path: "maps/loose.bsp".to_string(),
                source: "/corpus/q1/id1/maps/loose.bsp".to_string(),
                member_index: None,
            },
        ];
        let mut child = installed_product("child", GameFamily::Q1, "classic", "child", Some("q1-classic-id1"));
        child.expectation.id = "child-id".to_string();
        child.maps = vec![ContentMap {
            path: "maps/shared.bsp".to_string(),
            source: "/corpus/q1/child/maps/shared.bsp".to_string(),
            member_index: None,
        }];
        child.loose_root = Some("/corpus/q1/child".to_string());
        let catalog = test_catalog(vec![base, child]);
        let winners = catalog.maps_for("child-id").unwrap();
        let shared = winners.iter().find(|map| map.path == "maps/shared.bsp").unwrap();
        assert_eq!(shared.source, "/corpus/q1/child/maps/shared.bsp");
        // ZIP-family archives let the last duplicate win; PAK keeps the first.
        let dup = winners.iter().find(|map| map.path == "maps/dup.bsp").unwrap();
        assert_eq!(dup.member_index, Some(1));
        assert!(winners.iter().any(|map| map.path == "maps/loose.bsp"));
    }

    #[test]
    fn mounts_and_plans_cover_loose_and_archive_content() {
        let root = temp_root("mounts");
        let id1 = root.join("q1").join("id1");
        std::fs::create_dir_all(id1.join("maps")).unwrap();
        let pak = build_pak(&[("maps/paked.bsp", b"bsp-bytes")]);
        std::fs::write(id1.join("pak0.pak"), &pak).unwrap();
        std::fs::write(id1.join("autoexec.cfg"), b"exec\n").unwrap();
        let mut product = installed_product("q1-classic-id1", GameFamily::Q1, "classic", "id1", None);
        product.loose_root = Some(id1.to_string_lossy().into_owned());
        product.archives = vec![CatalogArchive {
            path: id1.join("pak0.pak").to_string_lossy().into_owned(),
            format: ArchiveFormat::Pak,
            entries: vec![CatalogArchiveEntry {
                path: "maps/paked.bsp".to_string(),
                ordinal: 0,
                byte_length: 9,
            }],
        }];
        product.maps = vec![
            ContentMap {
                path: "maps/paked.bsp".to_string(),
                source: id1.join("pak0.pak").to_string_lossy().into_owned(),
                member_index: Some(0),
            },
            ContentMap {
                path: "maps/loose.bsp".to_string(),
                source: id1.join("maps").join("loose.bsp").to_string_lossy().into_owned(),
                member_index: None,
            },
        ];
        let catalog = InstalledCatalog::new(
            root.to_string_lossy().into_owned(),
            vec![product.clone()],
            Vec::new(),
            3,
            None,
        )
        .unwrap();
        let mounts = catalog.mounts_for(product.id.as_str()).unwrap();
        assert_eq!(mounts.len(), 2);
        assert!(matches!(mounts[0], ContentMount::Archive(_)));
        assert!(matches!(mounts[1], ContentMount::Loose(_)));
        assert_eq!(mounts[0].identity().generation, 3);
        assert_eq!(
            catalog.read(product.id.as_str(), "maps/paked.bsp").unwrap(),
            b"bsp-bytes"
        );
        assert_eq!(catalog.read(product.id.as_str(), "autoexec.cfg").unwrap(), b"exec\n");
        assert!(catalog.authored_starts_for(product.id.as_str()).unwrap().is_none());
        let selection = MountPlanSelection {
            id: create_mount_plan_id("test", "plan").unwrap(),
            assets: product.id.clone(),
            geometry: product.id.clone(),
            rules: None,
            explicit_presentation: false,
            additional: Vec::new(),
        };
        let plan = catalog.create_mount_plan(&selection).unwrap();
        assert!(plan.prefix_orders.is_empty());
        assert_eq!(plan.default_order.len(), 2);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn discovery_installs_loose_and_archive_products() {
        let root = temp_root("discover");
        let id1 = root.join("q1").join("id1");
        std::fs::create_dir_all(id1.join("maps")).unwrap();
        let empty = build_pak(&[]);
        std::fs::write(id1.join("pak0.pak"), &empty).unwrap();
        std::fs::write(id1.join("pak1.pak"), &empty).unwrap();
        std::fs::write(id1.join("maps").join("start.bsp"), b"bsp").unwrap();
        let options = DiscoverContentOptions::new(root.clone());
        let catalog = discover_installed_content(&options).unwrap();
        let product = catalog.product("q1-classic-id1").unwrap();
        assert_eq!(product.availability, ProductAvailability::Installed);
        assert!(product.maps.iter().any(|map| map.path == "maps/start.bsp"));
        assert!(product.diagnostics.is_empty());
        let hipnotic = catalog.product("q1-classic-hipnotic").unwrap();
        assert!(matches!(hipnotic.availability, ProductAvailability::Missing { .. }));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn discovery_reports_broken_archives_as_requirements() {
        let root = temp_root("broken");
        let id1 = root.join("q1").join("id1");
        std::fs::create_dir_all(&id1).unwrap();
        let empty = build_pak(&[]);
        std::fs::write(id1.join("pak0.pak"), &empty).unwrap();
        std::fs::write(id1.join("pak1.pak"), &empty).unwrap();
        std::fs::write(id1.join("broken.pk3"), b"not an archive").unwrap();
        let options = DiscoverContentOptions::new(root.clone());
        let catalog = discover_installed_content(&options).unwrap();
        let product = catalog.product("q1-classic-id1").unwrap();
        assert_eq!(product.diagnostics.len(), 1);
        assert!(product.diagnostics[0].starts_with("broken.pk3: "));
        match &product.availability {
            ProductAvailability::Missing { requirements } => {
                assert_eq!(requirements, &product.diagnostics);
            }
            other => panic!("unexpected availability: {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn discovery_checks_map_witnesses_and_bases() {
        let root = temp_root("witness");
        let baseq2 = root.join("q2").join("baseq2");
        std::fs::create_dir_all(&baseq2).unwrap();
        std::fs::write(baseq2.join("pak0.pak"), build_pak(&[])).unwrap();
        let mut options = DiscoverContentOptions::new(root.clone());
        options.discover_mods = false;
        let catalog = discover_installed_content(&options).unwrap();
        let base = catalog.product("q2-classic-baseq2").unwrap();
        match &base.availability {
            ProductAvailability::Missing { requirements } => {
                assert!(requirements.iter().any(|requirement| requirement == "maps/base1.bsp"));
            }
            other => panic!("unexpected availability: {other:?}"),
        }
        std::fs::write(baseq2.join("pak0.pak"), build_pak(&[("maps/base1.bsp", b"bsp")])).unwrap();
        let catalog = discover_installed_content(&options).unwrap();
        assert_eq!(
            catalog.product("q2-classic-baseq2").unwrap().availability,
            ProductAvailability::Installed
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn discovery_finds_mods_and_remote_overlays() {
        let root = temp_root("mods");
        let mymod = root.join("q2").join("mymod");
        std::fs::create_dir_all(&mymod).unwrap();
        std::fs::write(mymod.join("pak0.pak"), build_pak(&[])).unwrap();
        let mut options = DiscoverContentOptions::new(root.clone());
        options.discover_mods = true;
        let catalog = discover_installed_content(&options).unwrap();
        let product = catalog.product("q2-classic-mymod").unwrap();
        assert_eq!(product.expectation.campaign, "mymod");
        match &product.availability {
            ProductAvailability::Missing { requirements } => {
                assert_eq!(requirements, &vec!["base product q2-classic-baseq2".to_string()]);
            }
            other => panic!("unexpected availability: {other:?}"),
        }
        options.remote_content = Some(RemoteContentSelection {
            base: RemoteContentBase::Q2ClassicBaseq2,
            directory: "coopmod".to_string(),
        });
        let catalog = discover_installed_content(&options).unwrap();
        let overlay = catalog.product("q2-classic-baseq2-mod-coopmod").unwrap();
        // Overlays skip archive requirements but still need their base.
        match &overlay.availability {
            ProductAvailability::Missing { requirements } => {
                assert_eq!(requirements, &vec!["base product q2-classic-baseq2".to_string()]);
            }
            other => panic!("unexpected availability: {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn discovery_reads_user_content_and_descriptions() {
        let root = temp_root("user");
        let user = temp_root("user-content");
        let user_id1 = user.join("q1").join("id1").join("maps");
        std::fs::create_dir_all(&user_id1).unwrap();
        std::fs::write(user_id1.join("usermap.bsp"), b"bsp").unwrap();
        let mymod = root.join("q3a").join("mymod");
        std::fs::create_dir_all(mymod.join("maps")).unwrap();
        std::fs::write(mymod.join("description.txt"), b"My Mod!\0ignored").unwrap();
        let mut options = DiscoverContentOptions::new(root.clone());
        options.user_content_root = Some(user.clone());
        let catalog = discover_installed_content(&options).unwrap();
        let product = catalog.product("q1-classic-id1").unwrap();
        assert!(product.user_content.is_some());
        assert!(product.maps.iter().any(|map| map.path == "maps/usermap.bsp"));
        let found_mod = catalog.product("q3-classic-mymod").unwrap();
        assert_eq!(found_mod.expectation.title, "My Mod!");
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&user);
    }

    #[test]
    fn discovery_rejects_bad_generations_and_duplicates() {
        let root = temp_root("bad-options");
        let mut options = DiscoverContentOptions::new(root.clone());
        options.generation = u64::MAX;
        assert!(discover_installed_content(&options).is_err());
        options.generation = 0;
        options.products = Some(vec![
            expectation("dup-a", GameFamily::Q1, "classic", "id1", None),
            expectation("dup-b", GameFamily::Q1, "classic", "id1", None),
        ]);
        assert!(discover_installed_content(&options).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn authored_starts_resolve_from_archives() {
        let root = temp_root("starts");
        let baseq2 = root.join("q2").join("rerelease").join("baseq2");
        std::fs::create_dir_all(&baseq2).unwrap();
        let mapdb = br#"{"episodes": [{"id": "baseq2", "command": "newgame", "name": "B", "activity": "a"}],
            "maps": [{"episode": "baseq2", "bsp": "base1", "title": "T", "sp": true}]}"#;
        std::fs::write(
            baseq2.join("pak0.pak"),
            build_pak(&[("maps/base1.bsp", b"bsp"), ("mapdb.json", mapdb)]),
        )
        .unwrap();
        let mut options = DiscoverContentOptions::new(root.clone());
        options.discover_mods = false;
        let catalog = discover_installed_content(&options).unwrap();
        assert_eq!(
            catalog.product("q2-rerelease-baseq2").unwrap().availability,
            ProductAvailability::Installed
        );
        let starts = catalog.authored_starts_for("q2-rerelease-baseq2").unwrap().unwrap();
        assert_eq!(starts.starts.len(), 1);
        assert_eq!(starts.starts[0].path, "maps/base1.bsp");
        assert_eq!(starts.episode.as_ref().unwrap().id, "baseq2");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn managed_addons_hide_and_title() {
        let root = temp_root("addons");
        let segment = format!("qd_{}_{}", "ab".repeat(10), "cd".repeat(10));
        let directory = format!("q1/{segment}");
        assert!(!managed_addon_hidden(&root, "q1/plain").unwrap());
        assert!(managed_addon_title(&root, "q1/plain").unwrap().is_none());
        let install = root.join("q1").join(&segment);
        std::fs::create_dir_all(&install).unwrap();
        std::fs::write(install.join(".quaddicted.json"), r#"{"title": "Managed"}"#).unwrap();
        assert!(!managed_addon_hidden(&root, &directory).unwrap());
        assert_eq!(
            managed_addon_title(&root, &directory).unwrap().as_deref(),
            Some("Managed")
        );
        let removed = root.join(".addons").join("removed").join("q1").join(&segment);
        std::fs::create_dir_all(&removed).unwrap();
        assert!(managed_addon_hidden(&root, &directory).unwrap());
        let _ = std::fs::remove_dir_all(&root);
    }

    // Compat seam tests (`launch.ts`, `weapon-behaviors.ts`,
    // `qvm-weapon-behaviors.ts`, `native-weapon-behaviors.ts`).

    fn seam_content() -> ContentId {
        create_content_id(&ContentIdentity {
            family: GameFamily::Q1,
            edition: "classic".to_string(),
            package: "id1".to_string(),
            revision: "v1".to_string(),
        })
        .unwrap()
    }

    fn seam_reference(namespace: &str, name: &str, content: &ContentId) -> ProviderReference {
        ProviderReference {
            provider: ProviderId::new(namespace, name),
            content: content.clone(),
        }
    }

    fn seam_preset(content: &ContentId) -> LaunchPreset {
        let official = seam_reference("q1", "official", content);
        LaunchPreset {
            id: RecipeId("recipe:seam:v1".to_string()),
            weapon_behaviors: Vec::new(),
            mods: Vec::new(),
            map: MapSelection {
                geometry: ResourceRequest {
                    content: content.clone(),
                    path: "maps/test.bsp".to_string(),
                },
                entities: official.clone(),
            },
            campaign: CampaignSelection::None,
            movement: official.clone(),
            character: CharacterSelection {
                definition: official.clone(),
                appearance: official.clone(),
            },
            weapons: Vec::new(),
            equipment: disabled_equipment(),
            enemies: EnemySelection::MapDefined,
            presentation: PresentationSelection {
                doppler: crate::contract::DopplerSelection::Disabled,
                environment: EnvironmentSelection::Disabled,
                assets: content.clone(),
                hud: official.clone(),
                effects: official.clone(),
                audio: official,
            },
            engine_behavior: seam_reference("q1", "engine", content),
            combat: seam_reference("q1", "combat", content),
            inventory: seam_reference("q1", "inventory", content),
            r#match: seam_reference("q1", "match", content),
            transition: seam_reference("q1", "transition", content),
            execution: Vec::new(),
            timing: Vec::new(),
            ordering: FrameOrdering::Native {
                clock: ClockProfile::Q2Classic,
            },
        }
    }

    struct StubWeapons {
        timing: Vec<ProviderTiming>,
    }

    impl LaunchWeaponSources for StubWeapons {
        fn canonical_weapon_source(
            &self,
            _map: &ProviderReference,
            weapon: &ProviderReference,
            _catalog: &InstalledCatalog,
        ) -> Result<ProviderReference, CatalogError> {
            Ok(weapon.clone())
        }

        fn selected_weapon_resources(
            &self,
            _map: &ProviderReference,
            _weapons: &[ProviderReference],
            _catalog: &InstalledCatalog,
        ) -> Result<Vec<ResourceRequest>, CatalogError> {
            Ok(Vec::new())
        }

        fn selected_weapon_timing(
            &self,
            _map: &ProviderReference,
            _weapons: &[ProviderReference],
            _catalog: &InstalledCatalog,
        ) -> Result<Vec<ProviderTiming>, CatalogError> {
            Ok(self.timing.clone())
        }

        fn admit_weapon_timing(
            &self,
            timing: &mut Vec<ProviderTiming>,
            weapon: &ProviderTiming,
        ) -> Result<(), CatalogError> {
            if !timing.iter().any(|entry| entry.provider == weapon.provider) {
                timing.push(weapon.clone());
            }
            Ok(())
        }

        fn weapon_provider_ids(&self) -> Vec<ProviderId> {
            vec![
                ProviderId::new("q1", "weapons/classic/id1"),
                ProviderId::new("q1", "weapons/rerelease/id1"),
                ProviderId::new("q1", "weapons/classic/hipnotic"),
                ProviderId::new("q1", "weapons/rerelease/hipnotic"),
                ProviderId::new("q2", "weapons/classic/baseq2"),
                ProviderId::new("q2", "weapons/rerelease/baseq2"),
            ]
        }
    }

    struct StubCompat(QvmAbiProfile);

    impl LaunchQvmCompatibility for StubCompat {
        fn read_qvm_compatibility(
            &self,
            _mounts: &dyn BehaviorMounts,
            _artifact_path: &str,
            _digest: &ContentDigest,
            _role: QvmCompatRole,
        ) -> Result<QvmAbiProfile, CatalogError> {
            Ok(self.0)
        }
    }

    fn loose_product(root: &std::path::Path, content: &ContentId) -> CatalogProduct {
        CatalogProduct {
            id: content.clone(),
            expectation: expectation("seam-game", GameFamily::Q1, "classic", "id1", None),
            availability: ProductAvailability::Installed,
            archives: Vec::new(),
            loose_root: Some(root.to_string_lossy().into_owned()),
            user_content: None,
            maps: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn bare_product(id: &str, content: &ContentId) -> CatalogProduct {
        CatalogProduct {
            id: content.clone(),
            expectation: expectation(id, GameFamily::Q1, "classic", "id1", None),
            availability: ProductAvailability::Installed,
            archives: Vec::new(),
            loose_root: None,
            user_content: None,
            maps: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn launch_choice_selects_preset_and_overrides() {
        let content = seam_content();
        let preset = seam_preset(&content);
        let choice = preset_choice(preset.id.clone());
        assert!(matches!(choice.map, LaunchSelection::Preset));
        let selected = select_launch(&choice, &preset, None).unwrap();
        assert_eq!(selected.id, preset.id);
        assert_eq!(selected.preset, preset.id);
        assert_eq!(selected.map, preset.map);
        let other = seam_reference("q1", "other", &content);
        let mut overridden = preset_choice(preset.id.clone());
        overridden.movement = LaunchSelection::Selected(other.clone());
        let selected = select_launch(&overridden, &preset, Some(&RecipeId("recipe:seam:v2".to_string()))).unwrap();
        assert_eq!(selected.id.as_str(), "recipe:seam:v2");
        assert_eq!(selected.movement, other);
        let mut mismatched = preset_choice(RecipeId("recipe:other:v1".to_string()));
        mismatched.preset = RecipeId("recipe:other:v1".to_string());
        let error = select_launch(&mismatched, &preset, None).unwrap_err();
        assert!(error.to_string().contains("does not match supplied preset"), "{error}");
    }

    #[test]
    fn launch_prepare_rejects_conflicting_execution() {
        let content = seam_content();
        let mut preset = seam_preset(&content);
        let owner = seam_reference("q1", "official", &content);
        let module = |implementation: &str| ExecutionModule::Builtin {
            owner: owner.clone(),
            implementation: ProviderId::new("ts", implementation),
            role: ModuleRole::ServerGame,
            api: crate::contract::SourceModuleApi::Q2ClassicGame,
        };
        preset.execution = vec![module("a"), module("b")];
        let catalog = test_catalog(vec![bare_product("seam-game", &content)]);
        let options = ResolveLaunchOptions {
            choice: &preset_choice(preset.id.clone()),
            preset: &preset,
            catalog: &catalog,
            id: None,
            mounts: None,
        };
        let error = prepare_launch_mount_plan(&options, &StubWeapons { timing: Vec::new() }).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Conflicting execution modules for q1:official/server-game"),
            "{error}"
        );
    }

    #[test]
    fn launch_resolves_geometry_execution_and_ordering() {
        for (profile, server, client, ui) in [
            (QvmAbiProfile::Modern, 8u8, 4u8, 6u8),
            (QvmAbiProfile::Legacy116n, 7u8, 3u8, 4u8),
        ] {
            let root = temp_root("launch");
            std::fs::create_dir_all(root.join("maps")).unwrap();
            std::fs::create_dir_all(root.join("vm")).unwrap();
            std::fs::write(root.join("maps").join("test.bsp"), b"map-bytes").unwrap();
            std::fs::write(root.join("vm").join("qagame.qvm"), b"qvm-bytes").unwrap();
            let content = seam_content();
            let mut preset = seam_preset(&content);
            let owner = seam_reference("q1", "official", &content);
            preset.execution = vec![
                ExecutionModule::Builtin {
                    owner: owner.clone(),
                    implementation: ProviderId::new("ts", "game"),
                    role: ModuleRole::ServerGame,
                    api: crate::contract::SourceModuleApi::Q2ClassicGame,
                },
                ExecutionModule::Qvm {
                    owner: seam_reference("q3", "server", &content),
                    artifact: ResourceRequest {
                        content: content.clone(),
                        path: "vm/qagame.qvm".to_string(),
                    },
                    role: ModuleRole::ServerGame,
                    api: Q3ApiIdentity::Qagame(7),
                },
                ExecutionModule::Qvm {
                    owner: seam_reference("q3", "client", &content),
                    artifact: ResourceRequest {
                        content: content.clone(),
                        path: "vm/qagame.qvm".to_string(),
                    },
                    role: ModuleRole::ClientGame,
                    api: Q3ApiIdentity::Cgame(3),
                },
                ExecutionModule::Qvm {
                    owner: seam_reference("q3", "ui", &content),
                    artifact: ResourceRequest {
                        content: content.clone(),
                        path: "vm/qagame.qvm".to_string(),
                    },
                    role: ModuleRole::Ui,
                    api: Q3ApiIdentity::Ui(6),
                },
            ];
            let weapon_provider = ProviderId::new("q9", "weapons/test");
            preset.timing = vec![
                ProviderTiming {
                    // Stock providers are filtered out of the preset timing.
                    provider: ProviderId::new("q1", "equipment/threewave-grapple"),
                    clock: ClockProfile::Q2Classic,
                    numeric: Q1_DONOR_PROFILE,
                },
                ProviderTiming {
                    provider: ProviderId::new("q1", "custom"),
                    clock: ClockProfile::Q2Classic,
                    numeric: Q1_DONOR_PROFILE,
                },
            ];
            let catalog = test_catalog(vec![loose_product(&root, &content)]);
            let options = ResolveLaunchOptions {
                choice: &preset_choice(preset.id.clone()),
                preset: &preset,
                catalog: &catalog,
                id: None,
                mounts: None,
            };
            let weapons = StubWeapons {
                timing: vec![ProviderTiming {
                    provider: weapon_provider.clone(),
                    clock: ClockProfile::Q2Classic,
                    numeric: Q1_DONOR_PROFILE,
                }],
            };
            let recipe = resolve_launch(&options, &weapons, &StubCompat(profile)).unwrap();
            assert_eq!(recipe.schema_version, 3);
            assert_eq!(recipe.map.geometry_content, content);
            assert_eq!(recipe.map.geometry.requested_path, "maps/test.bsp");
            assert_eq!(recipe.execution.len(), 4);
            assert!(matches!(recipe.execution[0], ExecutionModule::Builtin { .. }));
            assert!(
                matches!(recipe.execution[1], ExecutionModule::Qvm { api: Q3ApiIdentity::Qagame(v), .. } if v == server)
            );
            assert!(
                matches!(recipe.execution[2], ExecutionModule::Qvm { api: Q3ApiIdentity::Cgame(v), .. } if v == client)
            );
            assert!(matches!(recipe.execution[3], ExecutionModule::Qvm { api: Q3ApiIdentity::Ui(v), .. } if v == ui));
            assert!(recipe
                .resources
                .iter()
                .any(|resource| resource.requested_path == "maps/test.bsp"));
            assert!(recipe
                .resources
                .iter()
                .any(|resource| resource.requested_path == "vm/qagame.qvm"));
            // Single-source artifacts pin their mount order.
            assert!(recipe
                .mounts
                .prefix_orders
                .iter()
                .any(|order| order.prefix == "vm/qagame.qvm"));
            // Stock preset timing is replaced by surviving, equipment, monster, and weapon rows.
            assert_eq!(recipe.timing.len(), 2);
            assert_eq!(recipe.timing[0].provider, ProviderId::new("q1", "custom"));
            assert_eq!(recipe.timing[1].provider, weapon_provider);
            // Native ordering with weapon providers becomes mixed.
            match &recipe.ordering {
                FrameOrdering::Mixed { providers } => {
                    assert!(providers.contains(&ProviderId::new("q1", "official")));
                    assert!(providers.contains(&ProviderId::new("q1", "custom")));
                    assert!(providers.contains(&weapon_provider));
                }
                ordering => panic!("expected mixed ordering, got {ordering:?}"),
            }
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    #[test]
    fn launch_reports_missing_and_absent_resources() {
        let root = temp_root("launch-missing");
        std::fs::create_dir_all(root.join("maps")).unwrap();
        std::fs::write(root.join("maps").join("test.bsp"), b"map-bytes").unwrap();
        let content = seam_content();
        let other_content = create_content_id(&ContentIdentity {
            family: GameFamily::Q1,
            edition: "classic".to_string(),
            package: "rogue".to_string(),
            revision: "v1".to_string(),
        })
        .unwrap();
        let other = bare_product("other-game", &other_content);
        let catalog = test_catalog(vec![loose_product(&root, &content), other]);
        let preset = seam_preset(&content);
        let options = ResolveLaunchOptions {
            choice: &preset_choice(preset.id.clone()),
            preset: &preset,
            catalog: &catalog,
            id: None,
            mounts: None,
        };
        let prepared = prepare_launch_mount_plan(&options, &StubWeapons { timing: Vec::new() }).unwrap();
        let mounted = open_mount_plan(&prepared.plan, OpenMountOptions::default()).unwrap();
        let error = resolve_launch_resource(
            &catalog,
            &mounted,
            &ResourceRequest {
                content: content.clone(),
                path: "maps/absent.bsp".to_string(),
            },
            LaunchResourceKind::Map,
        )
        .unwrap_err();
        assert!(error.to_string().contains("Required resource is missing"), "{error}");
        // Bytes from another content's mount fail the provenance check.
        let error = resolve_launch_resource(
            &catalog,
            &mounted,
            &ResourceRequest {
                content: other_content.clone(),
                path: "maps/test.bsp".to_string(),
            },
            LaunchResourceKind::Map,
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("is absent from its selected content and base"),
            "{error}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    fn seam_digest() -> ContentDigest {
        create_content_digest(&"ab".repeat(32)).unwrap()
    }

    fn seam_module() -> ModuleIdentity {
        ModuleIdentity {
            id: ProviderId::new("q1", "gameplay"),
            artifact_path: "progs.dat".to_string(),
            digest: seam_digest(),
            revision: "v1".to_string(),
        }
    }

    fn seam_program() -> QcWeaponProgramSnapshot {
        // Function 1 stores function 2 into the think field at statement 1.
        QcWeaponProgramSnapshot {
            digest: seam_digest(),
            capability_error: None,
            functions: vec![
                QcWeaponFunction {
                    index: 0,
                    name: "<none>".to_string(),
                    first_statement: -1,
                    parameter_words: 0,
                },
                QcWeaponFunction {
                    index: 1,
                    name: "fire".to_string(),
                    first_statement: 0,
                    parameter_words: 0,
                },
                QcWeaponFunction {
                    index: 2,
                    name: "think".to_string(),
                    first_statement: 2,
                    parameter_words: 0,
                },
            ],
            statements: vec![
                QcWeaponStatement {
                    opcode: QcWeaponOpcode::Address,
                    a: 0,
                    b: 10,
                    c: 4,
                },
                QcWeaponStatement {
                    opcode: QcWeaponOpcode::StorePFn,
                    a: 11,
                    b: 4,
                    c: 0,
                },
            ],
            think_field_offset: Some(7),
            initial_global_words: vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 7, 2],
            function_globals: vec![QcFunctionGlobal {
                name: "think".to_string(),
                offset: 11,
            }],
        }
    }

    fn seam_metadata() -> SourceWeaponBehaviorMetadata {
        SourceWeaponBehaviorMetadata {
            id: "q1:rocket".to_string(),
            title: "Rocket".to_string(),
            artifact_digest: seam_digest(),
            role: ProjectileRole::Rocket,
            fire_function: "fire".to_string(),
            activation_function: None,
        }
    }

    #[test]
    fn qc_weapon_behaviors_resolve_and_catalog() {
        let module = seam_module();
        let program = seam_program();
        let supported = resolve_qc_weapon_behavior(&module, &program, &seam_metadata());
        match supported {
            WeaponBehaviorCompatibility::Supported { definition } => {
                assert_eq!(definition.id, "q1:rocket");
                assert_eq!(definition.role, ProjectileRole::Rocket);
                assert!(definition.activate.is_none());
                assert!(matches!(
                    definition.fire,
                    WeaponBehaviorCallback::Quakec { function_index: 1, .. }
                ));
            }
            verdict => panic!("expected supported, got {verdict:?}"),
        }
        let mut gated = seam_metadata();
        gated.activation_function = Some("think".to_string());
        match resolve_qc_weapon_behavior(&module, &program, &gated) {
            WeaponBehaviorCompatibility::Supported { definition } => {
                assert!(matches!(
                    definition.activate,
                    Some(WeaponBehaviorCallback::Quakec { function_index: 2, .. })
                ));
            }
            verdict => panic!("expected gated supported, got {verdict:?}"),
        }
        let mut foreign = seam_metadata();
        foreign.artifact_digest = create_content_digest(&"cd".repeat(32)).unwrap();
        assert!(matches!(
            resolve_qc_weapon_behavior(&module, &program, &foreign),
            WeaponBehaviorCompatibility::Unsupported { reason } if reason.contains("different source artifact")
        ));
        let incapable = QcWeaponProgramSnapshot {
            capability_error: Some("missing field".to_string()),
            ..program.clone()
        };
        assert!(matches!(
            resolve_qc_weapon_behavior(&module, &incapable, &seam_metadata()),
            WeaponBehaviorCompatibility::Unsupported { reason } if reason == "missing field"
        ));
        let mut missing = seam_metadata();
        missing.fire_function = "absent".to_string();
        assert!(matches!(
            resolve_qc_weapon_behavior(&module, &program, &missing),
            WeaponBehaviorCompatibility::Unsupported { reason } if reason.contains("absent from the source program")
        ));
        let mut catalog = WeaponBehaviorCatalog::new();
        let WeaponBehaviorCompatibility::Supported { definition } =
            resolve_qc_weapon_behavior(&module, &program, &seam_metadata())
        else {
            panic!("expected supported");
        };
        catalog.add(definition.clone()).unwrap();
        assert!(catalog
            .add(definition.clone())
            .unwrap_err()
            .to_string()
            .contains("Duplicate weapon behavior"));
        assert_eq!(catalog.for_role(ProjectileRole::Rocket).len(), 1);
        assert!(catalog.for_role(ProjectileRole::Grenade).is_empty());
        assert_eq!(catalog.require("q1:rocket").unwrap(), definition);
        assert!(catalog
            .require("q1:absent")
            .unwrap_err()
            .to_string()
            .contains("Unknown weapon behavior"));
    }

    #[test]
    fn qc_trajectory_inspection_reports_literal_stores() {
        let bindings = inspect_qc_trajectory_bindings(&seam_program());
        assert_eq!(
            bindings,
            vec![QcTrajectoryBindingInspection {
                producer_function: 1,
                think_function: 2,
                statement: 1
            }]
        );
        let no_think = QcWeaponProgramSnapshot {
            think_field_offset: None,
            ..seam_program()
        };
        assert!(inspect_qc_trajectory_bindings(&no_think).is_empty());
        let mut reassigned = seam_program();
        reassigned.statements.push(QcWeaponStatement {
            opcode: QcWeaponOpcode::StoreFn,
            a: 0,
            b: 11,
            c: 0,
        });
        assert!(inspect_qc_trajectory_bindings(&reassigned).is_empty());
    }

    struct StubMounts {
        files: HashMap<String, Vec<u8>>,
        digest: ContentDigest,
    }

    impl BehaviorMounts for StubMounts {
        fn open_behavior(&self, path: &str) -> Result<Option<OpenedResource>, CatalogError> {
            let Some(bytes) = self.files.get(path) else {
                return Ok(None);
            };
            let content = seam_content();
            Ok(Some(OpenedResource::new(
                ResolvedResourceReference {
                    id: crate::contract::ResourceId(format!("resource:seam:{path}")),
                    requested_path: path.to_string(),
                    provenance: ResourceProvenance::Loose {
                        mount: LooseMount {
                            identity: create_mount_identity(create_mount_id("seam", "stub").unwrap(), content, 0)
                                .unwrap(),
                            root_path: "/stub".to_string(),
                        },
                        member_path: path.to_string(),
                    },
                    digest: self.digest.clone(),
                    byte_length: bytes.len() as u64,
                    resolution: ResourceResolution::DefaultOrder {
                        plan: create_mount_plan_id("seam", "stub").unwrap(),
                        rank: 0,
                    },
                },
                bytes.clone(),
            )))
        }
    }

    #[test]
    fn qc_weapon_discovery_reads_declarations_or_reports_bindings() {
        let program = seam_program();
        let module = seam_module();
        let empty = StubMounts {
            files: HashMap::new(),
            digest: seam_digest(),
        };
        match discover_qc_weapon_behaviors(&empty, &module, &program).unwrap() {
            MountedWeaponBehaviorDiscovery::Undeclared { bindings, reason } => {
                assert_eq!(bindings.len(), 1);
                assert!(reason.contains("No authored weapon behavior declaration"), "{reason}");
            }
            discovery => panic!("expected undeclared, got {discovery:?}"),
        }
        let digest = seam_digest();
        let document = format!(
            r#"{{"version": 1, "behaviors": [
                {{"id": "q1:rocket", "title": "Rocket", "artifactDigest": "{digest}", "role": "rocket",
                  "aspect": "trajectory", "fireFunction": "fire"}},
                {{"id": "q1:missing", "title": "Missing", "artifactDigest": "{digest}", "role": "nail",
                  "aspect": "trajectory", "fireFunction": "absent"}}]}}"#,
        );
        let declared = StubMounts {
            files: HashMap::from([("weapon-behaviors.json".to_string(), document.into_bytes())]),
            digest: seam_digest(),
        };
        match discover_qc_weapon_behaviors(&declared, &module, &program).unwrap() {
            MountedWeaponBehaviorDiscovery::Declared { declarations } => {
                assert_eq!(declarations.len(), 2);
                assert!(matches!(declarations[0], WeaponBehaviorCompatibility::Supported { .. }));
                assert!(matches!(
                    declarations[1],
                    WeaponBehaviorCompatibility::Unsupported { .. }
                ));
            }
            discovery => panic!("expected declared, got {discovery:?}"),
        }
        for (name, bytes) in [
            ("bad json", b"{not json".to_vec()),
            ("bad version", br#"{"version": 2, "behaviors": []}"#.to_vec()),
            (
                "bad entry",
                br#"{"version": 1, "behaviors": [{"id": "no-namespace"}]}"#.to_vec(),
            ),
        ] {
            let mounts = StubMounts {
                files: HashMap::from([("weapon-behaviors.json".to_string(), bytes)]),
                digest: seam_digest(),
            };
            assert!(
                discover_qc_weapon_behaviors(&mounts, &module, &program).is_err(),
                "{name}"
            );
        }
        let bad_digest = br#"{"version": 1, "behaviors": [
            {"id": "q1:bad", "title": "Bad", "artifactDigest": "zzz", "role": "rocket",
             "aspect": "trajectory", "fireFunction": "fire"}]}"#;
        let mounts = StubMounts {
            files: HashMap::from([("weapon-behaviors.json".to_string(), bad_digest.to_vec())]),
            digest: seam_digest(),
        };
        let error = discover_qc_weapon_behaviors(&mounts, &module, &program).unwrap_err();
        assert!(error.to_string().contains("artifact digest"), "{error}");
        let duplicate = format!(
            r#"{{"version": 1, "behaviors": [
                {{"id": "q1:dup", "title": "A", "artifactDigest": "{digest}", "role": "rocket",
                  "aspect": "trajectory", "fireFunction": "fire"}},
                {{"id": "q1:dup", "title": "B", "artifactDigest": "{digest}", "role": "rocket",
                  "aspect": "trajectory", "fireFunction": "fire"}}]}}"#,
        );
        let mounts = StubMounts {
            files: HashMap::from([("weapon-behaviors.json".to_string(), duplicate.into_bytes())]),
            digest: seam_digest(),
        };
        let error = discover_qc_weapon_behaviors(&mounts, &module, &program).unwrap_err();
        assert!(
            error.to_string().contains("Duplicate declared weapon behavior"),
            "{error}"
        );
    }

    struct StubQvmService {
        bytecode: bool,
    }

    impl QvmWeaponBehaviorService<String, String> for StubQvmService {
        fn resolve_qvm_artifact(
            &self,
            _abi_profile: QvmAbiProfile,
            _bytes: &[u8],
            module: &ModuleIdentity,
        ) -> Result<QvmWeaponArtifactResolution<String>, CatalogError> {
            if self.bytecode {
                Ok(QvmWeaponArtifactResolution::Bytecode(module.artifact_path.clone()))
            } else {
                Ok(QvmWeaponArtifactResolution::Other)
            }
        }

        fn read_qvm_weapon_profile(&self, declaration: &SaveJson, _artifact: &String) -> Result<String, CatalogError> {
            match declaration.get("id") {
                Some(SaveJson::String(id)) => Ok(id.clone()),
                _ => Err(invalid("missing profile id")),
            }
        }

        fn qvm_weapon_profile_id(&self, profile: &String) -> String {
            profile.clone()
        }
    }

    fn qvm_declaration(id: &str) -> SaveJson {
        parse_save_json(&format!(
            r#"{{"artifactPath": "vm/qagame.qvm", "abiProfile": "q3-modern", "id": "{id}"}}"#,
        ))
        .unwrap()
    }

    #[test]
    fn qvm_weapon_behaviors_load_and_deduplicate() {
        let document =
            read_qvm_weapon_behavior_document(br#"{"version": 1, "profiles": [{"id": "a"}, {"id": "b"}]}"#).unwrap();
        assert_eq!(document.len(), 2);
        assert!(read_qvm_weapon_behavior_document(br#"{"version": 2, "profiles": []}"#).is_err());
        let provider = ProviderId::new("q3", "gameplay");
        let mounts = StubMounts {
            files: HashMap::from([
                ("vm/qagame.qvm".to_string(), b"bytecode".to_vec()),
                (
                    "qvm-weapon-behaviors.json".to_string(),
                    br#"{"version": 1, "profiles": [
                        {"artifactPath": "vm/qagame.qvm", "abiProfile": "q3-modern", "id": "qvm:rocket"},
                        {"artifactPath": "vm/qagame.qvm", "abiProfile": "q3-1.16n-base", "id": "qvm:rail"}]}"#
                        .to_vec(),
                ),
            ]),
            digest: seam_digest(),
        };
        let service = StubQvmService { bytecode: true };
        let loaded = load_qvm_weapon_behavior(&mounts, &provider, &qvm_declaration("qvm:rocket"), &service).unwrap();
        assert_eq!(loaded.artifact, "vm/qagame.qvm");
        assert_eq!(loaded.resource.requested_path, "vm/qagame.qvm");
        assert_eq!(loaded.profile, "qvm:rocket");
        let discovered = discover_qvm_weapon_behaviors(&mounts, &provider, &service)
            .unwrap()
            .unwrap();
        assert_eq!(discovered.len(), 2);
        let absent = StubMounts {
            files: HashMap::new(),
            digest: seam_digest(),
        };
        assert!(discover_qvm_weapon_behaviors(&absent, &provider, &service)
            .unwrap()
            .is_none());
        let error = load_qvm_weapon_behavior(&absent, &provider, &qvm_declaration("qvm:rocket"), &service).unwrap_err();
        assert!(error.to_string().contains("artifact is missing"), "{error}");
        let error = load_qvm_weapon_behavior(
            &mounts,
            &provider,
            &qvm_declaration("qvm:rocket"),
            &StubQvmService { bytecode: false },
        )
        .unwrap_err();
        assert!(error.to_string().contains("requires actual bytecode"), "{error}");
        let duplicated = StubMounts {
            files: HashMap::from([
                ("vm/qagame.qvm".to_string(), b"bytecode".to_vec()),
                (
                    "qvm-weapon-behaviors.json".to_string(),
                    br#"{"version": 1, "profiles": [
                        {"artifactPath": "vm/qagame.qvm", "abiProfile": "q3-modern", "id": "qvm:dup"},
                        {"artifactPath": "vm/qagame.qvm", "abiProfile": "q3-modern", "id": "qvm:dup"}]}"#
                        .to_vec(),
                ),
            ]),
            digest: seam_digest(),
        };
        let error = discover_qvm_weapon_behaviors(&duplicated, &provider, &service).unwrap_err();
        assert!(error.to_string().contains("Duplicate QVM weapon behavior"), "{error}");
    }

    fn native_declaration(artifact: &str, id: &str) -> NativeWeaponBehaviorDeclaration {
        use crate::contract::{
            NativeWeaponAllocate, NativeWeaponCalls, NativeWeaponClient, NativeWeaponCommand, NativeWeaponEntity,
            NativeWeaponEntry, NativeWeaponEquipped, NativeWeaponFree, NativeWeaponThink, NativeWeaponTime,
        };
        let entry = NativeWeaponEntry {
            rva: 16,
            registration: None,
        };
        NativeWeaponBehaviorDeclaration {
            version: 1,
            id: id.to_string(),
            title: id.to_string(),
            role: ProjectileRole::Rocket,
            artifact_path: artifact.to_string(),
            artifact_digest: seam_digest(),
            entity: NativeWeaponEntity {
                byte_length: 64,
                origin: 0,
                angles: 8,
                velocity: 16,
                client: 24,
                owner: 32,
                view_height: 40,
                generation: 44,
                next_think: 48,
                think_callback: 52,
                think_registration: 56,
                touch_callback: 60,
            },
            client: NativeWeaponClient {
                byte_length: 16,
                weapon: 0,
                view_angles: 4,
                forward: 8,
            },
            equipped_weapon: NativeWeaponEquipped {
                byte_length: 8,
                callback: 0,
                expected: entry.clone(),
            },
            time: NativeWeaponTime { rva: 32 },
            think: NativeWeaponThink {
                tag: 1,
                registration: crate::contract::NativeWeaponRegistrationLayout {
                    byte_length: 8,
                    name: 0,
                    tag: 4,
                    callback: 8,
                },
            },
            allocate: NativeWeaponAllocate { entry: entry.clone() },
            free: NativeWeaponFree { entry: entry.clone() },
            projectile_touch: entry.clone(),
            equip: NativeWeaponCalls {
                calls: vec![entry.clone()],
            },
            launch: NativeWeaponCalls {
                calls: vec![entry.clone()],
            },
            activate_rva: None,
            fire_rva: 48,
            initialization_classes: Vec::new(),
            equipment: Vec::new(),
            ammunition: NativeWeaponCommand {
                arguments: Vec::new(),
                tail: String::new(),
            },
            initial_cvars: Vec::new(),
            provisioning_cvars: Vec::new(),
        }
    }

    fn native_definition(id: &str, module: &ModuleIdentity) -> WeaponBehaviorDefinition {
        WeaponBehaviorDefinition {
            id: id.to_string(),
            title: id.to_string(),
            module: module.clone(),
            role: ProjectileRole::Rocket,
            activate: None,
            fire: WeaponBehaviorCallback::NativeArtifact {
                module: module.clone(),
                image_offset: 48,
                abi: crate::contract::NativeCallAbi::Native(crate::contract::NativeAbi::WindowsX86_64),
            },
        }
    }

    struct StubNativeService {
        definition: bool,
        builtin: bool,
    }

    impl NativeWeaponBehaviorService for StubNativeService {
        fn read_native_weapon_declaration(
            &self,
            value: &SaveJson,
        ) -> Result<NativeWeaponBehaviorDeclaration, CatalogError> {
            let reader = SaveReader::new(value);
            let artifact = reader.field("artifactPath").string().map_err(CatalogError::from)?;
            let id = reader.field("id").string().map_err(CatalogError::from)?;
            Ok(native_declaration(&artifact, &id))
        }

        fn native_weapon_definition(
            &self,
            declaration: &NativeWeaponBehaviorDeclaration,
            module: &ModuleIdentity,
            _image_bytes: &[u8],
        ) -> Result<Option<WeaponBehaviorDefinition>, CatalogError> {
            Ok(self.definition.then(|| native_definition(&declaration.id, module)))
        }

        fn builtin_rerelease_weapon_declaration(
            &self,
            _module: &ModuleIdentity,
        ) -> Result<Option<NativeWeaponBehaviorDeclaration>, CatalogError> {
            Ok(self
                .builtin
                .then(|| native_declaration("game_x64.dll", "native:blaster")))
        }
    }

    #[test]
    fn native_weapon_behaviors_load_document_and_builtin_fallback() {
        let document = read_native_weapon_behavior_document(br#"{"version": 1, "profiles": [{"id": "a"}]}"#).unwrap();
        assert_eq!(document.len(), 1);
        assert!(read_native_weapon_behavior_document(br#"{"version": 0, "profiles": []}"#).is_err());
        let provider = ProviderId::new("q2", "gameplay");
        let service = StubNativeService {
            definition: true,
            builtin: true,
        };
        let mounts = StubMounts {
            files: HashMap::from([
                ("game_x64.dll".to_string(), b"pe-bytes".to_vec()),
                (
                    "native-weapon-behaviors.json".to_string(),
                    br#"{"version": 1, "profiles": [
                        {"artifactPath": "game_x64.dll", "id": "native:blaster"},
                        {"artifactPath": "game_x64.dll", "id": "native:rail"}]}"#
                        .to_vec(),
                ),
            ]),
            digest: seam_digest(),
        };
        let value = parse_save_json(r#"{"artifactPath": "game_x64.dll", "id": "native:blaster"}"#).unwrap();
        let loaded = load_native_weapon_behavior(&mounts, &provider, &value, &service).unwrap();
        assert_eq!(loaded.definition.id, "native:blaster");
        assert_eq!(loaded.declaration.artifact_path, "game_x64.dll");
        let discovered = discover_native_weapon_behaviors(&mounts, &provider, &service)
            .unwrap()
            .unwrap();
        assert_eq!(discovered.len(), 2);
        // No document falls back to the stock game image.
        let fallback = StubMounts {
            files: HashMap::from([("game_x64.dll".to_string(), b"pe-bytes".to_vec())]),
            digest: seam_digest(),
        };
        let discovered = discover_native_weapon_behaviors(&fallback, &provider, &service)
            .unwrap()
            .unwrap();
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].definition.id, "native:blaster");
        let empty = StubMounts {
            files: HashMap::new(),
            digest: seam_digest(),
        };
        assert!(discover_native_weapon_behaviors(&empty, &provider, &service)
            .unwrap()
            .is_none());
        let no_builtin = StubNativeService {
            definition: true,
            builtin: false,
        };
        assert!(discover_native_weapon_behaviors(&fallback, &provider, &no_builtin)
            .unwrap()
            .is_none());
        // Missing artifacts and empty definitions fail the load.
        let error = load_native_weapon_behavior(&empty, &provider, &value, &service).unwrap_err();
        assert!(error.to_string().contains("artifact is missing"), "{error}");
        let no_definition = StubNativeService {
            definition: false,
            builtin: true,
        };
        let error = load_native_weapon_behavior(&mounts, &provider, &value, &no_definition).unwrap_err();
        assert!(error.to_string().contains("no executable definition"), "{error}");
        let duplicated = StubMounts {
            files: HashMap::from([
                ("game_x64.dll".to_string(), b"pe-bytes".to_vec()),
                (
                    "native-weapon-behaviors.json".to_string(),
                    br#"{"version": 1, "profiles": [
                        {"artifactPath": "game_x64.dll", "id": "native:dup"},
                        {"artifactPath": "game_x64.dll", "id": "native:dup"}]}"#
                        .to_vec(),
                ),
            ]),
            digest: seam_digest(),
        };
        let error = discover_native_weapon_behaviors(&duplicated, &provider, &service).unwrap_err();
        assert!(
            error.to_string().contains("Duplicate native weapon behavior"),
            "{error}"
        );
    }
}
