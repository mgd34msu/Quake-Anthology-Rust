//! Cgame body profile (`presentation/cgame-body-profile.ts`).

use qa_guest::error::GuestError;
use qa_guest::qvm::cgame_body::{
    QvmBodyReference, QvmBodySubmission, QvmBodyWhen, QvmSceneBodyMesh, QvmSceneBodyMeshCall,
};
use qa_guest::qvm::game_data::{QvmArtifact, QvmRole};
use qa_guest::qvm::mod_presentation::QvmBodyPart;

use crate::mounts::MountedContent;
use crate::paths::normalize_resource_path;
use crate::value::{parse_save_json, SaveReader, ValueError};

fn mapped(error: ValueError) -> GuestError {
    GuestError::invalid(error.to_string())
}

fn locals(entries: &[i32], player: i32, mesh: QvmSceneBodyMesh) -> Vec<QvmBodySubmission> {
    entries
        .iter()
        .map(|entry| QvmBodySubmission {
            entry: *entry as usize,
            actor_argument: 0,
            entity_number_offset: 0,
            reference: QvmBodyReference::Locals,
            when: None,
            mesh: (*entry == player).then(|| mesh.clone()),
        })
        .collect()
}

fn read_declared_body(artifact: &QvmArtifact, bytes: &[u8]) -> Result<Vec<QvmBodySubmission>, GuestError> {
    let text = String::from_utf8_lossy(bytes);
    let document = parse_save_json(&text).map_err(mapped)?;
    let reader = SaveReader::at(&document, "cgame-presentation.json");
    reader.field("version").literal_i64(1).map_err(mapped)?;
    let declared_path = reader.field("artifactPath").string().map_err(mapped)?;
    let normalized = normalize_resource_path(&declared_path).map_err(|error| GuestError::invalid(error.to_string()))?;
    let declared_digest = reader.field("artifactDigest").string().map_err(mapped)?;
    if normalized != artifact.module.artifact_path || declared_digest != artifact.module.digest {
        return Err(mapped(
            reader.fail("presentation declaration belongs to different cgame bytes"),
        ));
    }
    reader
        .field("bodySubmissions")
        .list(|item| {
            let storage = item.field("reference");
            let kind = storage.field("kind").choice_str(&["locals", "argument"])?;
            let condition = item.field("when");
            let mesh = item.field("mesh");
            Ok(QvmBodySubmission {
                entry: item.field("entry").integer(0)? as usize,
                actor_argument: item.field("actorArgument").integer(0)? as usize,
                entity_number_offset: item.field("entityNumberOffset").integer(0)? as usize,
                reference: if kind == "locals" {
                    QvmBodyReference::Locals
                } else {
                    QvmBodyReference::Argument {
                        index: storage.field("index").integer(0)? as usize,
                    }
                },
                when: if condition.is_missing() {
                    None
                } else {
                    Some(QvmBodyWhen {
                        argument: condition.field("argument").integer(0)? as usize,
                        equals: condition.field("equals").integer(i64::MIN)? as i32,
                    })
                },
                mesh: if mesh.is_missing() {
                    None
                } else {
                    let parts = mesh.field("parts");
                    Some(QvmSceneBodyMesh {
                        entry: mesh.field("entry").integer(0)? as usize,
                        entity_argument: mesh.field("entityArgument").integer(0)? as i32,
                        state_argument: mesh.field("stateArgument").integer(0)? as i32,
                        shader_offset: mesh.field("shaderOffset").integer(0)? as i32,
                        parts: if parts.is_missing() {
                            None
                        } else {
                            Some(parts.list(|row| {
                                let part = row.field("part").choice_str(&["body", "lower", "upper", "head"])?;
                                Ok(QvmSceneBodyMeshCall {
                                    call: row.field("call").integer(0)? as usize,
                                    part: match part.as_str() {
                                        "body" => QvmBodyPart::Body,
                                        "lower" => QvmBodyPart::Lower,
                                        "upper" => QvmBodyPart::Upper,
                                        _ => QvmBodyPart::Head,
                                    },
                                })
                            })?)
                        },
                    })
                },
            })
        })
        .map_err(mapped)
}

/// Entries verified from each artifact's `CG_AddCEntity` dispatch and local
/// `refEntity` allocations.
pub fn read_cgame_body_profile(
    artifact: &QvmArtifact,
    mounts: &MountedContent,
) -> Result<Option<Vec<QvmBodySubmission>>, GuestError> {
    if artifact.role != QvmRole::Cgame {
        return Ok(None);
    }
    let opened = mounts
        .open("cgame-presentation.json", |_| true)
        .map_err(|error| GuestError::invalid(error.to_string()))?;
    if let Some(opened) = opened {
        return read_declared_body(artifact, &opened.bytes).map(Some);
    }
    match artifact.module.digest.as_str() {
        "sha256:a4744482c9b93852cc71f4d7ce03b3e4337e5d89844d27d272c2c16d74df07fa" => Ok(Some(locals(
            &[
                45608, 81284, 46022, 47069, 47502, 47708, 47822, 48149, 47699, 45882, 45800,
            ],
            81284,
            QvmSceneBodyMesh {
                entry: 80824,
                entity_argument: 0,
                state_argument: 1,
                shader_offset: 112,
                parts: Some(vec![
                    QvmSceneBodyMeshCall {
                        call: 82085,
                        part: QvmBodyPart::Lower,
                    },
                    QvmSceneBodyMeshCall {
                        call: 82484,
                        part: QvmBodyPart::Upper,
                    },
                    QvmSceneBodyMeshCall {
                        call: 82781,
                        part: QvmBodyPart::Head,
                    },
                ]),
            },
        ))),
        "sha256:14858804fb98609ed8b3b3c3b825f0a7cb544063f7e43735c884cd5e4a51157c" => Ok(Some(locals(
            &[36612, 62937, 36804, 38561, 39712, 39826, 39870, 39534, 40520],
            62937,
            QvmSceneBodyMesh {
                entry: 61925,
                entity_argument: 0,
                state_argument: 1,
                shader_offset: 112,
                parts: Some(vec![
                    QvmSceneBodyMeshCall {
                        call: 63314,
                        part: QvmBodyPart::Lower,
                    },
                    QvmSceneBodyMeshCall {
                        call: 63546,
                        part: QvmBodyPart::Upper,
                    },
                    QvmSceneBodyMeshCall {
                        call: 63772,
                        part: QvmBodyPart::Head,
                    },
                ]),
            },
        ))),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{create_content_id, ContentIdentity, GameFamily};
    use crate::q3::test_support::{fixture_artifact, mount_loose_dir};

    const FIRST: &str = "sha256:a4744482c9b93852cc71f4d7ce03b3e4337e5d89844d27d272c2c16d74df07fa";

    fn mounted(name: &str, files: &[(&str, &str)]) -> MountedContent {
        let root = std::env::temp_dir().join(format!("qa-q3-cgame-body-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        for (name, text) in files {
            std::fs::write(root.join(name), text).unwrap();
        }
        let content = create_content_id(&ContentIdentity {
            family: GameFamily::Q3,
            edition: "classic".to_string(),
            package: "baseq3".to_string(),
            revision: "v1".to_string(),
        })
        .unwrap();
        mount_loose_dir(&root, &content)
    }

    #[test]
    fn profiles_both_cgame_artifacts() {
        let mounts = mounted("builtin", &[]);
        let first = fixture_artifact(FIRST, QvmRole::Cgame, 64, &[]);
        let submissions = read_cgame_body_profile(&first, &mounts).unwrap().unwrap();
        assert_eq!(submissions.len(), 11);
        let player = submissions.iter().find(|submission| submission.entry == 81284).unwrap();
        let mesh = player.mesh.as_ref().unwrap();
        assert_eq!(mesh.entry, 80824);
        assert_eq!(mesh.parts.as_ref().unwrap().len(), 3);
        assert!(submissions
            .iter()
            .filter(|submission| submission.entry != 81284)
            .all(|submission| submission.mesh.is_none()));
        let second = fixture_artifact(
            "sha256:14858804fb98609ed8b3b3c3b825f0a7cb544063f7e43735c884cd5e4a51157c",
            QvmRole::Cgame,
            64,
            &[],
        );
        let submissions = read_cgame_body_profile(&second, &mounts).unwrap().unwrap();
        assert_eq!(submissions.len(), 9);
        let other = fixture_artifact("sha256:other", QvmRole::Cgame, 64, &[]);
        assert!(read_cgame_body_profile(&other, &mounts).unwrap().is_none());
        let qagame = fixture_artifact(FIRST, QvmRole::Qagame, 64, &[]);
        assert!(read_cgame_body_profile(&qagame, &mounts).unwrap().is_none());
    }

    #[test]
    fn reads_declared_presentations() {
        let declaration = format!(
            r#"{{"version": 1, "artifactPath": "vm/qagame.qvm", "artifactDigest": "{FIRST}",
            "bodySubmissions": [
              {{"entry": 1, "actorArgument": 0, "entityNumberOffset": 0,
                "reference": {{"kind": "locals"}}}},
              {{"entry": 2, "actorArgument": 1, "entityNumberOffset": 4,
                "reference": {{"kind": "argument", "index": 3}},
                "when": {{"argument": 1, "equals": 7}},
                "mesh": {{"entry": 5, "entityArgument": 0, "stateArgument": 1, "shaderOffset": 112,
                  "parts": [{{"call": 9, "part": "head"}}]}}}}]}}"#
        );
        let mounts = mounted("declared", &[("cgame-presentation.json", &declaration)]);
        let artifact = fixture_artifact(FIRST, QvmRole::Cgame, 64, &[]);
        let submissions = read_cgame_body_profile(&artifact, &mounts).unwrap().unwrap();
        assert_eq!(submissions.len(), 2);
        assert_eq!(submissions[0].reference, QvmBodyReference::Locals);
        assert_eq!(submissions[1].reference, QvmBodyReference::Argument { index: 3 });
        assert_eq!(submissions[1].when, Some(QvmBodyWhen { argument: 1, equals: 7 }));
        let mesh = submissions[1].mesh.as_ref().unwrap();
        assert_eq!(mesh.parts.as_ref().unwrap()[0].part, QvmBodyPart::Head);
        let mismatch = mounted(
            "mismatch",
            &[(
                "cgame-presentation.json",
                r#"{"version": 1, "artifactPath": "vm/other.qvm", "artifactDigest": "sha256:other", "bodySubmissions": []}"#,
            )],
        );
        assert!(read_cgame_body_profile(&artifact, &mismatch).is_err());
    }
}
