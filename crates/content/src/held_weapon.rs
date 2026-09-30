//! Held weapon declarations.
//!
//! Donor: `src/content/held-weapon.ts`.

use thiserror::Error;

use crate::contract::{is_content_digest, ContentDigest, HeldWeaponDeclaration, HeldWeaponModel, HeldWeaponPart};
use crate::model_attachment::{parse_declaration_json, read_model_grip, ModelAttachmentError};
use crate::paths::{normalize_resource_path, PathError};
use crate::value::{SaveReader, ValueError};

/// Held weapon failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HeldWeaponError {
    /// Malformed declaration (carries the detail).
    #[error("{0}")]
    BadSave(String),
    /// Invalid resource path.
    #[error("{0}")]
    BadPath(String),
}

impl From<ValueError> for HeldWeaponError {
    fn from(error: ValueError) -> Self {
        Self::BadSave(error.to_string())
    }
}

impl From<PathError> for HeldWeaponError {
    fn from(error: PathError) -> Self {
        Self::BadPath(error.to_string())
    }
}

impl From<ModelAttachmentError> for HeldWeaponError {
    fn from(error: ModelAttachmentError) -> Self {
        Self::BadSave(error.to_string())
    }
}

/// Read a held weapon declaration (`readHeldWeaponDeclaration`).
pub fn read_held_weapon_declaration(reader: &SaveReader) -> Result<HeldWeaponDeclaration, HeldWeaponError> {
    if reader.field("kind").choice_str(&["none", "model"])? == "none" {
        return Ok(HeldWeaponDeclaration::None);
    }
    let model = reader.field("model");
    let part = model.field("part");
    let digest = model.field("digest");
    let subset = if part.is_missing() {
        None
    } else {
        let digests: Vec<String> = part.field("digests").list(|value| -> Result<String, HeldWeaponError> {
            let parsed = value.string()?;
            if !is_content_digest(&parsed) {
                return Err(HeldWeaponError::from(value.fail("held model requires a SHA256 digest")));
            }
            Ok(parsed)
        })?;
        let vertices: Vec<i64> = part.field("vertices").list(|value| value.integer(0))?;
        let mut distinct = vertices.clone();
        distinct.sort_unstable();
        distinct.dedup();
        if digests.is_empty() || vertices.is_empty() || distinct.len() != vertices.len() {
            return Err(part
                .fail("held model subset requires source digests and distinct vertices")
                .into());
        }
        #[allow(clippy::cast_precision_loss)]
        let subset = HeldWeaponPart {
            digests,
            vertices: vertices.into_iter().map(|vertex| vertex as f64).collect(),
        };
        Some(subset)
    };
    let path = normalize_resource_path(&model.field("path").string()?)?;
    #[allow(clippy::cast_precision_loss)]
    let reference_frame = model.field("referenceFrame").integer(0)? as f64;
    let grip = read_model_grip(&model.field("grip"))?;
    let digest = if digest.is_missing() {
        None
    } else {
        let parsed = digest.string()?;
        if !is_content_digest(&parsed) {
            return Err(digest.fail("held model requires a SHA256 digest").into());
        }
        Some(ContentDigest(parsed))
    };
    let fallback_field = model.field("fallback");
    let fallback = if fallback_field.is_missing() {
        None
    } else {
        Some(normalize_resource_path(&fallback_field.string()?)?)
    };
    Ok(HeldWeaponDeclaration::Model(HeldWeaponModel {
        digest,
        path,
        reference_frame,
        grip,
        fallback,
        part: subset,
    }))
}

/// Read a held weapon declaration file (`readHeldWeaponFile`).
pub fn read_held_weapon_file(bytes: &[u8]) -> Result<HeldWeaponDeclaration, HeldWeaponError> {
    let value = parse_declaration_json(bytes)?;
    let reader = SaveReader::new(&value);
    reader.field("version").literal_i64(1)?;
    read_held_weapon_declaration(&reader)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{arr, num, obj, str, SaveJson};

    fn vector(x: f64, y: f64, z: f64) -> SaveJson {
        obj(vec![("x", num(x)), ("y", num(y)), ("z", num(z))])
    }

    fn grip() -> SaveJson {
        obj(vec![
            (
                "axis",
                arr(vec![
                    vector(1.0, 0.0, 0.0),
                    vector(0.0, 1.0, 0.0),
                    vector(0.0, 0.0, 1.0),
                ]),
            ),
            ("origin", vector(0.0, 0.0, 0.0)),
        ])
    }

    fn digest() -> String {
        format!("sha256:{}", "cd".repeat(32))
    }

    #[test]
    fn reads_none_and_model_declarations() {
        let none = obj(vec![("kind", str("none"))]);
        assert_eq!(
            read_held_weapon_declaration(&SaveReader::new(&none)).unwrap(),
            HeldWeaponDeclaration::None
        );
        let model = obj(vec![
            ("kind", str("model")),
            (
                "model",
                obj(vec![
                    ("path", str("models/weapons/railgun.md3")),
                    ("referenceFrame", num(2.0)),
                    ("grip", grip()),
                    ("digest", str(&digest())),
                    ("fallback", str("models/weapons/shotgun.md3")),
                    (
                        "part",
                        obj(vec![
                            ("digests", arr(vec![str(&digest())])),
                            ("vertices", arr(vec![num(1.0), num(4.0)])),
                        ]),
                    ),
                ]),
            ),
        ]);
        match read_held_weapon_declaration(&SaveReader::new(&model)).unwrap() {
            HeldWeaponDeclaration::Model(model) => {
                assert_eq!(model.path, "models/weapons/railgun.md3");
                assert_eq!(model.reference_frame, 2.0);
                assert_eq!(model.digest.as_ref().unwrap().as_str(), digest());
                assert_eq!(model.fallback.as_deref(), Some("models/weapons/shotgun.md3"));
                let part = model.part.unwrap();
                assert_eq!(part.digests, vec![digest()]);
                assert_eq!(part.vertices, vec![1.0, 4.0]);
            }
            HeldWeaponDeclaration::None => panic!("expected model declaration"),
        }
    }

    #[test]
    fn rejects_bad_subsets_digests_and_paths() {
        // Duplicate subset vertices fail.
        let duplicate = obj(vec![
            ("kind", str("model")),
            (
                "model",
                obj(vec![
                    ("path", str("models/gun.md3")),
                    ("referenceFrame", num(0.0)),
                    ("grip", grip()),
                    (
                        "part",
                        obj(vec![
                            ("digests", arr(vec![str(&digest())])),
                            ("vertices", arr(vec![num(1.0), num(1.0)])),
                        ]),
                    ),
                ]),
            ),
        ]);
        let error = read_held_weapon_declaration(&SaveReader::new(&duplicate)).unwrap_err();
        assert!(error.to_string().contains("distinct vertices"), "{error}");
        // Empty subset digests fail.
        let empty = obj(vec![
            ("kind", str("model")),
            (
                "model",
                obj(vec![
                    ("path", str("models/gun.md3")),
                    ("referenceFrame", num(0.0)),
                    ("grip", grip()),
                    (
                        "part",
                        obj(vec![("digests", arr(vec![])), ("vertices", arr(vec![num(1.0)]))]),
                    ),
                ]),
            ),
        ]);
        assert!(read_held_weapon_declaration(&SaveReader::new(&empty)).is_err());
        // Non-digest model digest fails.
        let bad_digest = obj(vec![
            ("kind", str("model")),
            (
                "model",
                obj(vec![
                    ("path", str("models/gun.md3")),
                    ("referenceFrame", num(0.0)),
                    ("grip", grip()),
                    ("digest", str("nope")),
                ]),
            ),
        ]);
        let error = read_held_weapon_declaration(&SaveReader::new(&bad_digest)).unwrap_err();
        assert!(error.to_string().contains("SHA256 digest"), "{error}");
        // Invalid model path surfaces as a path failure.
        let bad_path = obj(vec![
            ("kind", str("model")),
            (
                "model",
                obj(vec![
                    ("path", str("../escape.md3")),
                    ("referenceFrame", num(0.0)),
                    ("grip", grip()),
                ]),
            ),
        ]);
        assert!(matches!(
            read_held_weapon_declaration(&SaveReader::new(&bad_path)).unwrap_err(),
            HeldWeaponError::BadPath(_)
        ));
    }

    #[test]
    fn reads_declaration_files() {
        let file = format!(
            r#"{{"version":1,"kind":"model","model":{{"path":"models/gun.md3","referenceFrame":0,"digest":"{}","grip":{{"axis":[{{"x":1,"y":0,"z":0}},{{"x":0,"y":1,"z":0}},{{"x":0,"y":0,"z":1}}],"origin":{{"x":0,"y":0,"z":0}}}}}}}}"#,
            digest()
        );
        match read_held_weapon_file(file.as_bytes()).unwrap() {
            HeldWeaponDeclaration::Model(model) => {
                assert_eq!(model.path, "models/gun.md3");
                assert!(model.fallback.is_none());
                assert!(model.part.is_none());
            }
            HeldWeaponDeclaration::None => panic!("expected model declaration"),
        }
        assert!(read_held_weapon_file(br#"{"version":2,"kind":"none"}"#).is_err());
        assert!(read_held_weapon_file(b"not json").is_err());
    }
}
