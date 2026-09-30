//! LRCTF grapple profile (`src/content/q3/equipment/lrctf-grapple-profile.ts`).

use qa_guest::error::GuestError;
use qa_guest::qvm::game_data::{ProfileReader, ProfileValue, QvmArtifact};
use qa_guest::qvm::grapple_profile::{read_qvm_grapple_profile, QvmGrappleProfile};

/// LRCTF 1.2 pak02 qagame digest.
pub const LRCTF_GRAPPLE_DIGEST: &str = "sha256:b9e396cf5ed2b913548cd92e2b0886ad5992653c8903fa3f9ed0b1f4167ca43e";

/// LRCTF 1.2 pak02 vm/qagame.qvm. Entries and offsets come from its own
/// bytecode.
pub fn lrctf_grapple_profile(artifact: &QvmArtifact) -> Result<Option<QvmGrappleProfile>, GuestError> {
    if artifact.module.digest != LRCTF_GRAPPLE_DIGEST {
        return Ok(None);
    }
    let declaration = ProfileValue::record(vec![
        ("version", ProfileValue::Int(1)),
        ("id", ProfileValue::Str("lrctf-1.2".to_string())),
        ("title", ProfileValue::Str("LRCTF (Quake 3)".to_string())),
        ("artifactDigest", ProfileValue::Str(artifact.module.digest.clone())),
        ("artifactPath", ProfileValue::Str(artifact.module.artifact_path.clone())),
        ("abiProfile", ProfileValue::Str("q3-modern".to_string())),
        ("entityStride", ProfileValue::Int(856)),
        ("clientStride", ProfileValue::Int(872)),
        (
            "fields",
            ProfileValue::record(vec![
                ("inuse", ProfileValue::Int(520)),
                ("client", ProfileValue::Int(516)),
                ("parent", ProfileValue::Int(600)),
                ("target", ProfileValue::Int(784)),
                ("mover", ProfileValue::Int(836)),
                ("health", ProfileValue::Int(748)),
                ("takedamage", ProfileValue::Int(752)),
                ("eventTime", ProfileValue::Int(552)),
                ("freeAfterEvent", ProfileValue::Int(556)),
                ("hook", ProfileValue::Int(840)),
            ]),
        ),
        (
            "globals",
            ProfileValue::record(vec![
                ("time", ProfileValue::Int(998864)),
                ("frame", ProfileValue::Int(998860)),
                ("movement", ProfileValue::Int(1008980)),
                ("forward", ProfileValue::Int(1008836)),
                ("groundPlane", ProfileValue::Int(1008884)),
            ]),
        ),
        (
            "callbacks",
            ProfileValue::record(vec![
                ("allocate", ProfileValue::Int(178797)),
                ("free", ProfileValue::Int(179014)),
                ("fire", ProfileValue::Int(183076)),
                ("release", ProfileValue::Int(183171)),
                ("forceRelease", ProfileValue::Int(183171)),
                ("missile", ProfileValue::Int(151164)),
                ("follow", ProfileValue::Int(183577)),
                ("think", ProfileValue::Int(5362)),
                ("pull", ProfileValue::Int(15874)),
                ("moveMoverHooks", ProfileValue::Int(184569)),
                ("damage", ProfileValue::Int(140358)),
                ("sameTeam", ProfileValue::Int(169306)),
                ("playerMove", ProfileValue::Int(22369)),
            ]),
        ),
        ("pullingFlag", ProfileValue::Int(2048)),
        ("fireArguments", ProfileValue::Array(vec![])),
        (
            "movement",
            ProfileValue::record(vec![
                ("byteLength", ProfileValue::Int(16)),
                ("words", ProfileValue::Array(vec![])),
            ]),
        ),
        ("initialCvars", ProfileValue::record(vec![])),
        ("eventLifetimeMilliseconds", ProfileValue::Int(300)),
        ("grappleDamageMethod", ProfileValue::Int(23)),
        (
            "presentation",
            ProfileValue::record(vec![
                (
                    "projectileModel",
                    ProfileValue::Str("models/weapons3/hook/hook1.md3".to_string()),
                ),
                (
                    "viewModel",
                    ProfileValue::Str("models/weapons3/hook/bit1.md3".to_string()),
                ),
                ("weaponIndex", ProfileValue::Int(10)),
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
                                ("x", ProfileValue::Int(0)),
                                ("y", ProfileValue::Int(0)),
                                ("z", ProfileValue::Int(0)),
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
                ("viewAttachments", ProfileValue::Array(vec![])),
                (
                    "cable",
                    ProfileValue::record(vec![
                        ("kind", ProfileValue::Str("shader".to_string())),
                        ("path", ProfileValue::Str("grapplerope".to_string())),
                        ("width", ProfileValue::Int(16)),
                    ]),
                ),
                (
                    "fireSound",
                    ProfileValue::Str("sound/grapple/midevil/grfire.wav".to_string()),
                ),
                (
                    "attachSound",
                    ProfileValue::Str("sound/grapple/midevil/grhit.wav".to_string()),
                ),
                ("releaseSound", ProfileValue::Null),
                ("pullSound", ProfileValue::Null),
                ("hangSound", ProfileValue::Null),
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

    fn lrctf_artifact() -> QvmArtifact {
        fixture_artifact(
            LRCTF_GRAPPLE_DIGEST,
            QvmRole::Qagame,
            1_008_984,
            &[
                178797, 179014, 183076, 183171, 151164, 183577, 5362, 15874, 184569, 140358, 169306, 22369,
            ],
        )
    }

    #[test]
    fn reads_lrctf_profile() {
        let profile = lrctf_grapple_profile(&lrctf_artifact()).unwrap().unwrap();
        assert_eq!(profile.id, "lrctf-1.2");
        assert_eq!((profile.entity_stride, profile.client_stride), (856, 872));
        assert_eq!(profile.fields.mover, Some(836));
        assert_eq!(profile.callbacks.follow, Some(183577));
        assert_eq!(profile.callbacks.move_mover_hooks, Some(184569));
        assert!(profile.fire_arguments.is_empty());
        assert_eq!(profile.grapple_damage_method, 23);
        assert_eq!(profile.event_lifetime_ms, 300);
        assert!(matches!(profile.presentation.cable, QvmCable::Shader { .. }));
        assert_eq!(profile.presentation.weapon_index, 10);
    }

    #[test]
    fn rejects_other_digests_and_bad_bytes() {
        let other = fixture_artifact("sha256:other", QvmRole::Qagame, 1_008_984, &[]);
        assert!(lrctf_grapple_profile(&other).unwrap().is_none());
        let short = fixture_artifact(LRCTF_GRAPPLE_DIGEST, QvmRole::Qagame, 1024, &[]);
        assert!(lrctf_grapple_profile(&short).is_err());
    }
}
