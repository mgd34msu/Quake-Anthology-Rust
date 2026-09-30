//! Q3 input profile (`input-profile.ts`).

use qa_guest::qvm::game_data::QvmArtifact;
use qa_guest::qvm::game_input::{QvmInputDefinition, QvmInputEntries, QvmMovementModes};

use super::equipment::lrctf_grapple_profile::LRCTF_GRAPPLE_DIGEST;
use super::equipment::threewave_grapple_profile::THREEWAVE_GRAPPLE_DIGEST;

/// Original qagame bytecode; all pointers still come from
/// `G_LOCATE_GAME_DATA`.
#[must_use]
pub fn q3_input_profile(artifact: &QvmArtifact) -> Option<QvmInputDefinition> {
    let movement_modes = Some(QvmMovementModes {
        normal: 0,
        noclip: 1,
        freeze: 4,
    });
    match artifact.module.digest.as_str() {
        digest if digest == LRCTF_GRAPPLE_DIGEST => Some(QvmInputDefinition {
            module: artifact.module.clone(),
            entity_stride: 856,
            client_stride: 872,
            client_pointer: 516,
            intermission: vec![5, 6],
            movement_modes,
            entries: QvmInputEntries {
                client_think: 114858,
                run_client: 114917,
                client_spawn: 125027,
                move_entry: 22369,
                slice: 21620,
            },
        }),
        digest if digest == THREEWAVE_GRAPPLE_DIGEST => Some(QvmInputDefinition {
            module: artifact.module.clone(),
            entity_stride: 876,
            client_stride: 944,
            client_pointer: 516,
            intermission: vec![7, 8],
            movement_modes,
            entries: QvmInputEntries {
                client_think: 120292,
                run_client: 120383,
                client_spawn: 132015,
                move_entry: 35535,
                slice: 34707,
            },
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use qa_guest::qvm::game_data::QvmRole;

    use super::*;
    use crate::q3::test_support::fixture_artifact;

    #[test]
    fn profiles_both_qagames() {
        let lrctf = fixture_artifact(LRCTF_GRAPPLE_DIGEST, QvmRole::Qagame, 64, &[]);
        let profile = q3_input_profile(&lrctf).unwrap();
        assert_eq!(
            (profile.entity_stride, profile.client_stride, profile.client_pointer),
            (856, 872, 516)
        );
        assert_eq!(profile.intermission, vec![5, 6]);
        assert_eq!(profile.entries.client_spawn, 125027);
        let threewave = fixture_artifact(THREEWAVE_GRAPPLE_DIGEST, QvmRole::Qagame, 64, &[]);
        let profile = q3_input_profile(&threewave).unwrap();
        assert_eq!(profile.intermission, vec![7, 8]);
        assert_eq!(profile.entries.move_entry, 35535);
    }

    #[test]
    fn rejects_other_digests() {
        let other = fixture_artifact("sha256:other", QvmRole::Qagame, 64, &[]);
        assert!(q3_input_profile(&other).is_none());
    }
}
