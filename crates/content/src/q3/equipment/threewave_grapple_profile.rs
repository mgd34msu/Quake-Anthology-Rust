//! Threewave CTF grapple profile (`src/content/q3/equipment/threewave-grapple-profile.ts`).

use qa_guest::error::GuestError;
use qa_guest::qvm::game_data::{ProfileReader, ProfileValue, QvmArtifact};
use qa_guest::qvm::grapple_profile::{read_qvm_grapple_profile, QvmGrappleProfile};

/// Threewave 1.7 qagame digest.
pub const THREEWAVE_GRAPPLE_DIGEST: &str = "sha256:9751bad99a2d138f96a9b0436d2ea2d965b86214175dc33e4cea95e059419337";

/// Threewave 1.7 qagame: private layout and callbacks verified against these
/// executable bytes.
pub fn threewave_grapple_profile(artifact: &QvmArtifact) -> Result<Option<QvmGrappleProfile>, GuestError> {
    if artifact.module.digest != THREEWAVE_GRAPPLE_DIGEST {
        return Ok(None);
    }
    let declaration = ProfileValue::record(vec![
        ("version", ProfileValue::Int(1)),
        ("id", ProfileValue::Str("threewave-1.7".to_string())),
        ("title", ProfileValue::Str("Threewave CTF (Quake 3)".to_string())),
        ("artifactDigest", ProfileValue::Str(artifact.module.digest.clone())),
        ("artifactPath", ProfileValue::Str(artifact.module.artifact_path.clone())),
        ("abiProfile", ProfileValue::Str("q3-modern".to_string())),
        ("entityStride", ProfileValue::Int(876)),
        ("clientStride", ProfileValue::Int(944)),
        (
            "fields",
            ProfileValue::record(vec![
                ("inuse", ProfileValue::Int(520)),
                ("client", ProfileValue::Int(516)),
                ("parent", ProfileValue::Int(600)),
                ("target", ProfileValue::Int(768)),
                ("mover", ProfileValue::Null),
                ("health", ProfileValue::Int(732)),
                ("takedamage", ProfileValue::Int(736)),
                ("hook", ProfileValue::Int(816)),
                ("eventTime", ProfileValue::Int(552)),
                ("freeAfterEvent", ProfileValue::Int(556)),
            ]),
        ),
        (
            "globals",
            ProfileValue::record(vec![
                ("time", ProfileValue::Int(1077712)),
                ("frame", ProfileValue::Int(1077708)),
                ("movement", ProfileValue::Int(1091860)),
                ("forward", ProfileValue::Int(1091720)),
                ("groundPlane", ProfileValue::Int(1091768)),
            ]),
        ),
        (
            "callbacks",
            ProfileValue::record(vec![
                ("allocate", ProfileValue::Int(210993)),
                ("free", ProfileValue::Int(211210)),
                ("fire", ProfileValue::Int(217563)),
                ("release", ProfileValue::Int(215035)),
                ("forceRelease", ProfileValue::Int(215169)),
                ("missile", ProfileValue::Int(177663)),
                ("follow", ProfileValue::Null),
                ("think", ProfileValue::Int(16897)),
                ("pull", ProfileValue::Int(29990)),
                ("moveMoverHooks", ProfileValue::Null),
                ("damage", ProfileValue::Int(162405)),
                ("sameTeam", ProfileValue::Int(197341)),
                ("playerMove", ProfileValue::Int(35535)),
            ]),
        ),
        ("fireArguments", ProfileValue::Array(vec![ProfileValue::Int(0)])),
        (
            "movement",
            ProfileValue::record(vec![
                ("byteLength", ProfileValue::Int(240)),
                (
                    "words",
                    ProfileValue::Array(vec![
                        ProfileValue::record(vec![
                            ("offset", ProfileValue::Int(232)),
                            ("value", ProfileValue::Int(10)),
                        ]),
                        ProfileValue::record(vec![
                            ("offset", ProfileValue::Int(236)),
                            ("value", ProfileValue::Int(0)),
                        ]),
                    ]),
                ),
            ]),
        ),
        (
            "initialCvars",
            ProfileValue::record(vec![
                ("g_gametype", ProfileValue::Str("10".to_string())),
                ("g_lithium", ProfileValue::Str("0".to_string())),
                ("p_enablePortal", ProfileValue::Str("0".to_string())),
            ]),
        ),
        ("pullingFlag", ProfileValue::Int(2048)),
        ("eventLifetimeMilliseconds", ProfileValue::Int(100)),
        ("grappleDamageMethod", ProfileValue::Int(29)),
        (
            "presentation",
            ProfileValue::record(vec![
                (
                    "projectileModel",
                    ProfileValue::Str("models/weapons2/grapple/grapple_hook.md3".to_string()),
                ),
                (
                    "viewModel",
                    ProfileValue::Str("models/weapons2/grapple/grap.md3".to_string()),
                ),
                ("weaponIndex", ProfileValue::Int(11)),
                (
                    "viewAnchor",
                    ProfileValue::record(vec![
                        (
                            "path",
                            ProfileValue::Str("models/weapons2/shotgun/shotgun_hand.md3".to_string()),
                        ),
                        ("tag", ProfileValue::Str("tag_weapon".to_string())),
                        (
                            "offset",
                            ProfileValue::record(vec![
                                ("x", ProfileValue::Int(5)),
                                ("y", ProfileValue::Int(0)),
                                ("z", ProfileValue::Int(-1)),
                            ]),
                        ),
                        (
                            "fovOffset",
                            ProfileValue::record(vec![
                                ("above", ProfileValue::Int(90)),
                                ("scale", ProfileValue::Float(-0.2)),
                            ]),
                        ),
                    ]),
                ),
                (
                    "viewAttachments",
                    ProfileValue::Array(vec![ProfileValue::record(vec![
                        (
                            "path",
                            ProfileValue::Str("models/weapons2/grapple/grapple_hand.md3".to_string()),
                        ),
                        ("tag", ProfileValue::Str("tag_hook".to_string())),
                    ])]),
                ),
                (
                    "cable",
                    ProfileValue::record(vec![
                        ("kind", ProfileValue::Str("model".to_string())),
                        (
                            "flight",
                            ProfileValue::Str("models/weapons2/grapple/grapple1_cord_s.md3".to_string()),
                        ),
                        (
                            "pull",
                            ProfileValue::Str("models/weapons2/grapple/grapple1_cord_p.md3".to_string()),
                        ),
                        (
                            "hold",
                            ProfileValue::Str("models/weapons2/grapple/grapple1_cord_f.md3".to_string()),
                        ),
                        ("segmentLength", ProfileValue::Int(14)),
                    ]),
                ),
                (
                    "fireSound",
                    ProfileValue::Str("sound/cctf/grapple/grapple_fire.wav".to_string()),
                ),
                (
                    "attachSound",
                    ProfileValue::Str("sound/cctf/grapple/grapple_hit.wav".to_string()),
                ),
                ("releaseSound", ProfileValue::Null),
                (
                    "pullSound",
                    ProfileValue::Str("sound/cctf/grapple/grapple_pull.wav".to_string()),
                ),
                (
                    "hangSound",
                    ProfileValue::Str("sound/cctf/grapple/grapple_hang.wav".to_string()),
                ),
            ]),
        ),
    ]);
    read_qvm_grapple_profile(&ProfileReader::new(&declaration), artifact).map(Some)
}

#[cfg(test)]
mod tests {
    use qa_guest::qvm::game_data::QvmRole;
    use qa_guest::qvm::grapple_profile::QvmCable;

    use super::*;
    use crate::q3::test_support::fixture_artifact;

    fn threewave_artifact() -> QvmArtifact {
        fixture_artifact(
            THREEWAVE_GRAPPLE_DIGEST,
            QvmRole::Qagame,
            1_091_864,
            &[
                210993, 211210, 217563, 215035, 215169, 177663, 16897, 29990, 162405, 197341, 35535,
            ],
        )
    }

    #[test]
    fn reads_threewave_profile() {
        let profile = threewave_grapple_profile(&threewave_artifact()).unwrap().unwrap();
        assert_eq!(profile.id, "threewave-1.7");
        assert_eq!(profile.title, "Threewave CTF (Quake 3)");
        assert_eq!((profile.entity_stride, profile.client_stride), (876, 944));
        assert_eq!(profile.fields.mover, None);
        assert_eq!(profile.callbacks.follow, None);
        assert_eq!(profile.callbacks.move_mover_hooks, None);
        assert_eq!(profile.fire_arguments, vec![0]);
        assert_eq!(profile.movement.byte_length, 240);
        assert_eq!(profile.grapple_damage_method, 29);
        assert_eq!(profile.event_lifetime_ms, 100);
        assert_eq!(profile.presentation.weapon_index, 11);
        assert!(matches!(profile.presentation.cable, QvmCable::Model { .. }));
        assert_eq!(profile.presentation.release_sound, None);
        assert_eq!(
            profile.presentation.pull_sound.as_deref(),
            Some("sound/cctf/grapple/grapple_pull.wav")
        );
    }

    #[test]
    fn rejects_other_digests_and_bad_bytes() {
        let other = fixture_artifact("sha256:other", QvmRole::Qagame, 1_091_864, &[]);
        assert!(threewave_grapple_profile(&other).unwrap().is_none());
        let short = fixture_artifact(THREEWAVE_GRAPPLE_DIGEST, QvmRole::Qagame, 1024, &[]);
        assert!(threewave_grapple_profile(&short).is_err());
    }
}
