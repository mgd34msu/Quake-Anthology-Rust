//! Port of `src/compat/q2/native-primary-player-profile.ts`.
//! Bridges builtin player profiles: Xatrix classic and retail rerelease tables.

use super::native_primary_player::{NativePrimaryPlayerProfile, PlayerObjectives, SourcePrimaryMatch, SourceTeam};
use super::native_primary_reader::{CLASSIC_DIGEST, RETAIL_DIGEST};

/// Builtin player profile for a digest, or `None` when unknown.
#[must_use]
pub fn native_primary_player_profile(digest: &str) -> Option<NativePrimaryPlayerProfile> {
    if digest == CLASSIC_DIGEST {
        Some(NativePrimaryPlayerProfile {
            digest: CLASSIC_DIGEST.to_string(),
            match_profile: Some(SourcePrimaryMatch {
                score: 0xd88,
                teams: vec![],
            }),
            spawn: 0x312a0,
            objectives: PlayerObjectives::None,
            command_angles: 0xd8c,
            velocity: 0x178,
            forward: None,
        })
    } else if digest == RETAIL_DIGEST {
        Some(NativePrimaryPlayerProfile {
            digest: RETAIL_DIGEST.to_string(),
            match_profile: Some(SourcePrimaryMatch {
                score: 0x17c8,
                teams: vec![
                    SourceTeam {
                        source: "q2:1".to_string(),
                        team: "team:red".to_string(),
                        arguments: vec!["team".to_string(), "red".to_string()],
                    },
                    SourceTeam {
                        source: "q2:2".to_string(),
                        team: "team:blue".to_string(),
                        arguments: vec!["team".to_string(), "blue".to_string()],
                    },
                ],
            }),
            spawn: 0xd9050,
            objectives: PlayerObjectives::Entry(0x11eb10),
            command_angles: 0x17cc,
            velocity: 0x694,
            forward: Some(0x19a4),
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_xatrix_profile() {
        let profile = native_primary_player_profile(CLASSIC_DIGEST).expect("xatrix");
        assert_eq!(profile.spawn, 0x312a0);
        assert_eq!(profile.objectives, PlayerObjectives::None);
        assert_eq!(profile.forward, None);
        assert_eq!(profile.match_profile.expect("match").score, 0xd88);
    }

    #[test]
    fn resolves_retail_profile() {
        let profile = native_primary_player_profile(RETAIL_DIGEST).expect("retail");
        assert_eq!(profile.objectives, PlayerObjectives::Entry(0x11eb10));
        assert_eq!(profile.forward, Some(0x19a4));
        let match_profile = profile.match_profile.expect("match");
        assert_eq!(match_profile.score, 0x17c8);
        assert_eq!(match_profile.teams.len(), 2);
        assert_eq!(match_profile.teams[0].team, "team:red");
    }

    #[test]
    fn rejects_unknown_digests() {
        assert!(native_primary_player_profile("sha256:dead").is_none());
    }
}
