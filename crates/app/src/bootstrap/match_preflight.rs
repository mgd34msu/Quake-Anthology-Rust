//! Match preflight validation before simulation or renderer construction.
//!
//! Donor provenance: `src/app/bootstrap/match-preflight.ts`
//! (`preflightApplicationMatch`). The donor reads recipes, catalogs, and Q3
//! objective adapters owned by sibling lanes; this port takes those resolved
//! inputs as plain data and keeps the donor's decision order: guest modules
//! skip validation, Q3 resolves the game type and checks objectives, and Q1/Q2
//! map the match provider to rules and validate the map.

use crate::bootstrap::match_modes::{match_map_unavailable, MatchModeSelection, MatchRules};
use crate::options::GameMode;
use qa_content::contract::GameFamily;
use thiserror::Error;

/// Failure of match preflight validation.
#[derive(Debug, Error)]
pub enum MatchPreflightError {
    /// Selected match cannot start for the given reason.
    #[error("{0}")]
    Unavailable(String),
}

/// Resolved source-game expectation for preflight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightSource {
    /// Source game family.
    pub family: GameFamily,
    /// Product edition.
    pub edition: String,
    /// Campaign identifier.
    pub campaign: String,
}

/// Server-game execution module ownership for preflight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightExecution {
    /// Module role (`server-game` for the game module).
    pub role: String,
    /// Whether the module is a TypeScript module.
    pub typescript: bool,
}

/// Resolved content inputs for preflight validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightContent {
    /// Source game expectation.
    pub source: PreflightSource,
    /// Server-game execution module, when present.
    pub execution: Option<PreflightExecution>,
    /// Match provider (`q2:ctf`, `q1:horde`, ...).
    pub match_provider: String,
    /// Selected Q3 source program (`missionpack` or `baseq3`).
    pub q3_source_program: String,
    /// World entity classnames.
    pub classnames: Vec<String>,
}

/// Q3 objective adaptation outcome for the selected map and game type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreflightQ3Objectives {
    /// Objectives adapt cleanly.
    Ready,
    /// Required objective classnames are missing.
    MissingObjectives {
        /// Missing classnames.
        classnames: Vec<String>,
    },
    /// Product does not support the game type.
    UnsupportedMode {
        /// Rejected game type.
        game_type: i64,
    },
}

/// Configured source value (`g_gametype` for Q3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightSourceValue {
    /// Value name.
    pub name: String,
    /// Value text.
    pub value: String,
}

/// Resolve the Q3 game type: last configured `g_gametype`, else singleplayer 2, else 0.
#[must_use]
pub fn preflight_q3_game_type(mode: GameMode, source_values: &[PreflightSourceValue]) -> Option<i64> {
    let mut configured: Option<&str> = None;
    for value in source_values {
        if value.name == "g_gametype" {
            configured = Some(value.value.as_str());
        }
    }
    match configured {
        None => Some(if mode == GameMode::Singleplayer { 2 } else { 0 }),
        Some(text) => text
            .parse::<f64>()
            .ok()
            .map(|parsed| if parsed.is_nan() { 0 } else { parsed.trunc() as i64 }),
    }
}

/// Map a match provider to source match rules.
#[must_use]
pub fn preflight_match_rules(provider: &str) -> MatchRules {
    match provider {
        "q2:ctf" => MatchRules::Ctf,
        "q2:lmctf" => MatchRules::Lmctf,
        "q2:tag" => MatchRules::Tag,
        "q2:deathball" => MatchRules::Deathball,
        "q1:horde" => MatchRules::Horde,
        _ => MatchRules::Standard,
    }
}

/// Validate the configured source rules before constructing a simulation or renderer.
///
/// Guest modules own their mode definitions and admission; stock rules cannot
/// validate them, so anything but a TypeScript server-game module skips
/// validation. The Q3 objective outcome comes from the caller-owned adapter.
pub fn preflight_application_match(
    content: &PreflightContent,
    mode: GameMode,
    source_values: &[PreflightSourceValue],
    q3_objectives: &dyn Fn(&str, i64) -> PreflightQ3Objectives,
) -> Result<(), MatchPreflightError> {
    let typescript = content
        .execution
        .as_ref()
        .is_some_and(|execution| execution.role == "server-game" && execution.typescript);
    if !typescript {
        return Ok(());
    }
    if content.source.family == GameFamily::Q3 {
        let game_type = preflight_q3_game_type(mode, source_values).unwrap_or(0);
        let program = if content.q3_source_program == "missionpack" {
            "missionpack"
        } else {
            "baseq3"
        };
        match q3_objectives(program, game_type) {
            PreflightQ3Objectives::Ready => return Ok(()),
            PreflightQ3Objectives::MissingObjectives { classnames } => {
                return Err(MatchPreflightError::Unavailable(format!(
                    "Selected Q3 match map is missing: {}",
                    classnames.join(", ")
                )));
            }
            PreflightQ3Objectives::UnsupportedMode { game_type } => {
                return Err(MatchPreflightError::Unavailable(format!(
                    "Selected Q3 product does not support game type {game_type}"
                )));
            }
        }
    }
    let selection = MatchModeSelection {
        family: content.source.family,
        edition: content.source.edition.clone(),
        campaign: content.source.campaign.clone(),
        mode,
        rules: preflight_match_rules(&content.match_provider),
    };
    if let Some(reason) = match_map_unavailable(&selection, &content.classnames) {
        return Err(MatchPreflightError::Unavailable(reason));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content(family: GameFamily) -> PreflightContent {
        PreflightContent {
            source: PreflightSource {
                family,
                edition: "classic".to_owned(),
                campaign: "base".to_owned(),
            },
            execution: Some(PreflightExecution {
                role: "server-game".to_owned(),
                typescript: true,
            }),
            match_provider: "standard".to_owned(),
            q3_source_program: "baseq3".to_owned(),
            classnames: Vec::new(),
        }
    }

    #[test]
    fn guest_and_missing_modules_skip_validation() {
        let mut guest = content(GameFamily::Q2);
        guest.execution = Some(PreflightExecution {
            role: "server-game".to_owned(),
            typescript: false,
        });
        assert!(preflight_application_match(&guest, GameMode::Deathmatch, &[], &|_, _| {
            PreflightQ3Objectives::MissingObjectives {
                classnames: vec!["x".to_owned()],
            }
        })
        .is_ok());
        let mut absent = content(GameFamily::Q2);
        absent.execution = None;
        assert!(
            preflight_application_match(&absent, GameMode::Singleplayer, &[], &|_, _| {
                PreflightQ3Objectives::Ready
            })
            .is_ok()
        );
    }

    #[test]
    fn resolves_q3_game_type_from_last_value_or_mode() {
        assert_eq!(preflight_q3_game_type(GameMode::Singleplayer, &[]), Some(2));
        assert_eq!(preflight_q3_game_type(GameMode::Deathmatch, &[]), Some(0));
        let values = [
            PreflightSourceValue {
                name: "g_gametype".to_owned(),
                value: "3".to_owned(),
            },
            PreflightSourceValue {
                name: "g_gametype".to_owned(),
                value: "4".to_owned(),
            },
        ];
        assert_eq!(preflight_q3_game_type(GameMode::Singleplayer, &values), Some(4));
    }

    #[test]
    fn reports_q3_objective_failures() {
        let q3 = content(GameFamily::Q3);
        let err = preflight_application_match(&q3, GameMode::Deathmatch, &[], &|program, game_type| {
            assert_eq!(program, "baseq3");
            assert_eq!(game_type, 0);
            PreflightQ3Objectives::MissingObjectives {
                classnames: vec!["team_CTF_redflag".to_owned()],
            }
        })
        .expect_err("missing objectives");
        assert_eq!(err.to_string(), "Selected Q3 match map is missing: team_CTF_redflag");
        let err = preflight_application_match(&q3, GameMode::Deathmatch, &[], &|_, _| {
            PreflightQ3Objectives::UnsupportedMode { game_type: 9 }
        })
        .expect_err("unsupported mode");
        assert_eq!(err.to_string(), "Selected Q3 product does not support game type 9");
    }

    #[test]
    fn maps_providers_to_rules_and_validates_map() {
        assert_eq!(preflight_match_rules("q2:ctf"), MatchRules::Ctf);
        assert_eq!(preflight_match_rules("q2:lmctf"), MatchRules::Lmctf);
        assert_eq!(preflight_match_rules("q2:tag"), MatchRules::Tag);
        assert_eq!(preflight_match_rules("q2:deathball"), MatchRules::Deathball);
        assert_eq!(preflight_match_rules("q1:horde"), MatchRules::Horde);
        assert_eq!(preflight_match_rules("other"), MatchRules::Standard);
        let mut ctf = content(GameFamily::Q2);
        ctf.match_provider = "q2:ctf".to_owned();
        let err = preflight_application_match(&ctf, GameMode::Deathmatch, &[], &|_, _| PreflightQ3Objectives::Ready)
            .expect_err("missing flags");
        assert!(err.to_string().starts_with("CTF map is missing: "));
    }
}
