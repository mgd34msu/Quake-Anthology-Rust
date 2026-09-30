//! Q3 grapple profile dispatch (`grapple-profiles.ts`).

use qa_guest::error::GuestError;
use qa_guest::qvm::game_data::QvmArtifact;
use qa_guest::qvm::grapple_profile::QvmGrappleProfile;

use super::lrctf_grapple_profile::lrctf_grapple_profile;
use super::threewave_grapple_profile::threewave_grapple_profile;

/// Q3 grapple profile for a qualified artifact, LRCTF first.
pub fn q3_grapple_profile(artifact: &QvmArtifact) -> Result<Option<QvmGrappleProfile>, GuestError> {
    if let Some(profile) = lrctf_grapple_profile(artifact)? {
        return Ok(Some(profile));
    }
    threewave_grapple_profile(artifact)
}

#[cfg(test)]
mod tests {
    use qa_guest::qvm::game_data::QvmRole;

    use super::*;
    use crate::q3::equipment::lrctf_grapple_profile::LRCTF_GRAPPLE_DIGEST;
    use crate::q3::equipment::threewave_grapple_profile::THREEWAVE_GRAPPLE_DIGEST;
    use crate::q3::test_support::fixture_artifact;

    #[test]
    fn dispatches_lrctf_before_threewave() {
        let lrctf = fixture_artifact(
            LRCTF_GRAPPLE_DIGEST,
            QvmRole::Qagame,
            1_008_984,
            &[
                178797, 179014, 183076, 183171, 151164, 183577, 5362, 15874, 184569, 140358, 169306, 22369,
            ],
        );
        assert_eq!(q3_grapple_profile(&lrctf).unwrap().unwrap().id, "lrctf-1.2");
        let threewave = fixture_artifact(
            THREEWAVE_GRAPPLE_DIGEST,
            QvmRole::Qagame,
            1_091_864,
            &[
                210993, 211210, 217563, 215035, 215169, 177663, 16897, 29990, 162405, 197341, 35535,
            ],
        );
        assert_eq!(q3_grapple_profile(&threewave).unwrap().unwrap().id, "threewave-1.7");
        let other = fixture_artifact("sha256:other", QvmRole::Qagame, 64, &[]);
        assert!(q3_grapple_profile(&other).unwrap().is_none());
    }
}
