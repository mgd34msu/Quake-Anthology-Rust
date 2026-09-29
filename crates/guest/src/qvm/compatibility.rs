//! QVM compatibility declarations: per-module ABI profiles.
//!
//! Port of `src/compat/qvm/compatibility.ts`
//! (`parseQvmCompatibilityDeclaration`, `readQvmCompatibilityDeclaration`,
//! `readQvmCompatibility`). Every interface in a declaration belongs to exact
//! module bytes: role plus artifact path select the entry and the artifact
//! digest must match.
//!
//! Local mirrors (owned by other workers):
//!
//! - `QvmCompatibilityMounts` mirrors the `open` surface of the donor content
//!   mounts. The donor decodes JSON here; this workspace has no JSON codec in
//!   `qa-guest`'s dependencies, so decoding belongs to the content layer and
//!   the trait yields parsed [`SaveJson`](qa_world::save::value::SaveJson).
//! - `normalize_resource_path` mirrors the donor content path normalization
//!   (`qa-content` is not a `qa-guest` dependency).
//!
//! Sync-port note: the donor `read*` functions are async; here they are
//! synchronous.

use std::collections::HashSet;

use qa_world::save::value::{SaveJson, SaveReader};

use crate::error::GuestError;

use super::syscalls::{QvmAbiProfile, QvmRole};

/// Parsed compatibility declaration for one module.
#[derive(Debug, Clone)]
pub struct QvmCompatibilityDeclaration {
    /// Selected ABI profile.
    pub profile: QvmAbiProfile,
    /// Retained `primary` player interfaces (`qagame` only).
    pub primary: Option<SaveJson>,
    /// Retained `equipmentPresentation` (`cgame` only).
    pub equipment_presentation: Option<SaveJson>,
}

impl Default for QvmCompatibilityDeclaration {
    fn default() -> Self {
        Self {
            profile: QvmAbiProfile::Modern,
            primary: None,
            equipment_presentation: None,
        }
    }
}

/// Normalize a relative resource path: backslashes become slashes; empty
/// paths, NUL bytes, drive prefixes, and empty/dot/dot-dot segments fail.
pub fn normalize_resource_path(path: &str) -> Result<String, GuestError> {
    let normalized = path.replace('\\', "/");
    let drive = normalized.len() >= 2
        && normalized.as_bytes()[0].is_ascii_alphabetic()
        && normalized.as_bytes()[1] == b':';
    if normalized.is_empty()
        || normalized.contains('\0')
        || drive
        || normalized.split('/').any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(GuestError::invalid(format!("Invalid relative resource path: {path}")));
    }
    Ok(normalized)
}

fn is_digest(text: &str) -> bool {
    text.len() == 7 + 64
        && text.as_bytes()[..7] == *b"sha256:"
        && text.as_bytes()[7..].iter().all(|byte| byte.is_ascii_hexdigit())
}

fn role_from(text: &str) -> Option<QvmRole> {
    match text {
        "qagame" => Some(QvmRole::Qagame),
        "cgame" => Some(QvmRole::Cgame),
        "ui" => Some(QvmRole::Ui),
        _ => None,
    }
}

/// Parse a `qvm-compatibility.json` value for one module.
pub fn parse_qvm_compatibility_declaration(
    value: &SaveJson,
    artifact_path: &str,
    digest: &str,
    role: QvmRole,
) -> Result<QvmCompatibilityDeclaration, GuestError> {
    let reader = SaveReader::at(value, "qvm-compatibility.json");
    reader.field("version").literal_i64(1).map_err(GuestError::from)?;
    let mut seen = HashSet::new();
    let mut selected = QvmCompatibilityDeclaration::default();
    let entries: Vec<SaveReader<'_>> = reader
        .field("modules")
        .list(|entry| Ok::<_, GuestError>(entry))
        .map_err(GuestError::from)?;
    for entry in entries {
        let entry_role = entry.field("role").choice_str(&["qagame", "cgame", "ui"]).map_err(GuestError::from)?;
        let path = normalize_resource_path(&entry.field("artifactPath").string().map_err(GuestError::from)?)?;
        let key = format!("{entry_role}/{}", path.to_lowercase());
        if !seen.insert(key) {
            return Err(GuestError::from(entry.fail("duplicate module compatibility declaration")));
        }
        let entry_digest = entry.field("artifactDigest").string().map_err(GuestError::from)?;
        if !is_digest(&entry_digest) {
            return Err(GuestError::from(entry.fail("expected exact sha256 artifact digest")));
        }
        let profile = entry
            .field("profile")
            .choice_str(&["q3-modern", "q3-1.16n-base"])
            .map_err(GuestError::from)?;
        let primary = entry.field("primary");
        let equipment = entry.field("equipmentPresentation");
        if primary.value.is_some() && entry_role != "qagame" {
            return Err(GuestError::from(primary.fail("primary player interfaces belong to qagame")));
        }
        if equipment.value.is_some() && entry_role != "cgame" {
            return Err(GuestError::from(equipment.fail("equipment presentation belongs to cgame")));
        }
        if role_from(&entry_role) != Some(role) || path.to_lowercase() != artifact_path.to_lowercase() {
            continue;
        }
        if entry_digest != digest {
            return Err(GuestError::from(
                entry.fail("compatibility declaration belongs to different artifact bytes"),
            ));
        }
        selected = QvmCompatibilityDeclaration {
            profile: QvmAbiProfile::parse(&profile)?,
            primary: primary.value.cloned(),
            equipment_presentation: equipment.value.cloned(),
        };
    }
    Ok(selected)
}

/// Parse only the ABI profile for one module.
pub fn parse_qvm_compatibility(
    value: &SaveJson,
    artifact_path: &str,
    digest: &str,
    role: QvmRole,
) -> Result<QvmAbiProfile, GuestError> {
    parse_qvm_compatibility_declaration(value, artifact_path, digest, role).map(|declaration| declaration.profile)
}

/// Opened compatibility file: parsed JSON plus its resource reference.
#[derive(Debug, Clone)]
pub struct QvmCompatibilityFile {
    /// Parsed declaration JSON.
    pub json: SaveJson,
    /// Resource reference of the opened file.
    pub reference: String,
}

/// Content-mount surface for compatibility declarations.
pub trait QvmCompatibilityMounts {
    /// Open `path`, or `None` when it does not exist.
    fn open(&self, path: &str) -> Result<Option<QvmCompatibilityFile>, GuestError>;
}

/// Read the declaration for one module, defaulting to modern when absent.
pub fn read_qvm_compatibility_declaration(
    mounts: &dyn QvmCompatibilityMounts,
    artifact_path: &str,
    digest: &str,
    role: QvmRole,
) -> Result<(QvmCompatibilityDeclaration, Option<String>), GuestError> {
    let Some(opened) = mounts.open("qvm-compatibility.json")? else {
        return Ok((QvmCompatibilityDeclaration::default(), None));
    };
    let declaration = parse_qvm_compatibility_declaration(&opened.json, artifact_path, digest, role)?;
    Ok((declaration, Some(opened.reference)))
}

/// Read only the ABI profile for one module.
pub fn read_qvm_compatibility(
    mounts: &dyn QvmCompatibilityMounts,
    artifact_path: &str,
    digest: &str,
    role: QvmRole,
) -> Result<QvmAbiProfile, GuestError> {
    read_qvm_compatibility_declaration(mounts, artifact_path, digest, role)
        .map(|(declaration, _)| declaration.profile)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_world::save::value::{obj, str};

    fn declaration(modules: Vec<SaveJson>) -> SaveJson {
        obj(vec![
            ("version", SaveJson::Number(1.0)),
            ("modules", SaveJson::Array(modules)),
        ])
    }

    fn entry(role: &str, path: &str, digest: &str, profile: &str) -> SaveJson {
        obj(vec![
            ("role", str(role)),
            ("artifactPath", str(path)),
            ("artifactDigest", str(digest)),
            ("profile", str(profile)),
        ])
    }

    const DIGEST: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000000";

    #[test]
    fn selects_matching_module_entries() {
        let value = declaration(vec![
            entry("cgame", "vm/cgame.qvm", DIGEST, "q3-modern"),
            entry("qagame", "vm/qagame.qvm", DIGEST, "q3-1.16n-base"),
        ]);
        let declaration =
            parse_qvm_compatibility_declaration(&value, "vm/qagame.qvm", DIGEST, QvmRole::Qagame)
                .unwrap();
        assert_eq!(declaration.profile, QvmAbiProfile::Legacy116n);
        let profile =
            parse_qvm_compatibility(&value, "vm/cgame.qvm", DIGEST, QvmRole::Cgame).unwrap();
        assert_eq!(profile, QvmAbiProfile::Modern);
        let missing =
            parse_qvm_compatibility(&value, "vm/other.qvm", DIGEST, QvmRole::Qagame).unwrap();
        assert_eq!(missing, QvmAbiProfile::Modern);
    }

    #[test]
    fn rejects_duplicates_digest_mismatches_and_misplaced_interfaces() {
        let duplicate = declaration(vec![
            entry("qagame", "vm/qagame.qvm", DIGEST, "q3-modern"),
            entry("qagame", "VM/qagame.qvm", DIGEST, "q3-modern"),
        ]);
        assert!(parse_qvm_compatibility_declaration(&duplicate, "vm/qagame.qvm", DIGEST, QvmRole::Qagame).is_err());
        let wrong_digest = declaration(vec![entry("qagame", "vm/qagame.qvm", DIGEST, "q3-modern")]);
        assert!(parse_qvm_compatibility_declaration(
            &wrong_digest,
            "vm/qagame.qvm",
            "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            QvmRole::Qagame,
        )
        .is_err());
        let misplaced = obj(vec![
            ("version", SaveJson::Number(1.0)),
            (
                "modules",
                SaveJson::Array(vec![obj(vec![
                    ("role", str("cgame")),
                    ("artifactPath", str("vm/cgame.qvm")),
                    ("artifactDigest", str(DIGEST)),
                    ("profile", str("q3-modern")),
                    ("primary", str("nope")),
                ])]),
            ),
        ]);
        assert!(parse_qvm_compatibility(&misplaced, "vm/cgame.qvm", DIGEST, QvmRole::Cgame).is_err());
    }

    #[test]
    fn retains_primary_and_equipment_values() {
        let value = obj(vec![
            ("version", SaveJson::Number(1.0)),
            (
                "modules",
                SaveJson::Array(vec![obj(vec![
                    ("role", str("qagame")),
                    ("artifactPath", str("vm/qagame.qvm")),
                    ("artifactDigest", str(DIGEST)),
                    ("profile", str("q3-modern")),
                    ("primary", SaveJson::Number(7.0)),
                ])]),
            ),
        ]);
        let declaration =
            parse_qvm_compatibility_declaration(&value, "vm/qagame.qvm", DIGEST, QvmRole::Qagame)
                .unwrap();
        assert_eq!(declaration.primary, Some(SaveJson::Number(7.0)));
        assert_eq!(declaration.equipment_presentation, None);
    }

    struct MapMounts {
        file: Option<QvmCompatibilityFile>,
    }

    impl QvmCompatibilityMounts for MapMounts {
        fn open(&self, path: &str) -> Result<Option<QvmCompatibilityFile>, GuestError> {
            assert_eq!(path, "qvm-compatibility.json");
            Ok(self.file.clone())
        }
    }

    #[test]
    fn missing_files_default_to_modern() {
        let mounts = MapMounts { file: None };
        let (declaration, reference) =
            read_qvm_compatibility_declaration(&mounts, "vm/qagame.qvm", DIGEST, QvmRole::Qagame)
                .unwrap();
        assert_eq!(declaration.profile, QvmAbiProfile::Modern);
        assert_eq!(reference, None);
        let profile = read_qvm_compatibility(&mounts, "vm/qagame.qvm", DIGEST, QvmRole::Qagame).unwrap();
        assert_eq!(profile, QvmAbiProfile::Modern);
    }

    #[test]
    fn resource_paths_normalize_like_content() {
        assert_eq!(normalize_resource_path("vm\\qagame.qvm").unwrap(), "vm/qagame.qvm");
        assert!(normalize_resource_path("").is_err());
        assert!(normalize_resource_path("c:/x").is_err());
        assert!(normalize_resource_path("vm/../x").is_err());
    }
}
