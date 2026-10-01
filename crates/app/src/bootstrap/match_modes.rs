//! Match rules and map validation for configured source games.
//!
//! Donor provenance: `src/app/bootstrap/match-modes.ts`
//! (`MatchRules`, `MatchModeSelection`, `matchModeUnavailable`,
//! `matchMapUnavailable`). Direct port with no behavioral changes.

use qa_content::contract::GameFamily;

use crate::options::GameMode;

/// Source match rules beyond standard play.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MatchRules {
    /// Standard rules.
    Standard,
    /// Capture the flag.
    Ctf,
    /// Threewave CTF.
    Lmctf,
    /// Tag.
    Tag,
    /// DeathBall.
    Deathball,
    /// Horde mode.
    Horde,
}

/// Selected source game and match configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchModeSelection {
    /// Source game family.
    pub family: GameFamily,
    /// Product edition (`classic`, `rerelease`, ...).
    pub edition: String,
    /// Campaign identifier.
    pub campaign: String,
    /// Play mode.
    pub mode: GameMode,
    /// Match rules.
    pub rules: MatchRules,
}

/// Reason the selected rules are unavailable, or `None` when playable.
#[must_use]
pub fn match_mode_unavailable(selection: &MatchModeSelection) -> Option<String> {
    let MatchModeSelection {
        family,
        edition,
        campaign,
        mode,
        rules,
    } = selection;
    if *rules == MatchRules::Standard {
        return None;
    }
    if *rules == MatchRules::Horde {
        // Donor: `family !== "q1" || edition !== "rerelease" || campaign !== "mg1" && campaign !== "dopa"`.
        let wrong_campaign = campaign != "mg1" && campaign != "dopa";
        if *family != GameFamily::Q1 || edition != "rerelease" || wrong_campaign {
            return Some("Horde requires Quake rerelease Dimension of the Machine or Dimension of the Past".to_owned());
        }
        return if *mode == GameMode::Deathmatch {
            Some("Horde requires single player or cooperative mode".to_owned())
        } else {
            None
        };
    }
    if *family != GameFamily::Q2 {
        return Some("These match rules require a Quake II source game".to_owned());
    }
    if *mode != GameMode::Deathmatch {
        return Some("These match rules require deathmatch mode".to_owned());
    }
    if *rules == MatchRules::Ctf || *rules == MatchRules::Lmctf {
        return if edition != "classic" {
            Some("This CTF ruleset requires classic Quake II".to_owned())
        } else {
            None
        };
    }
    if edition != "rerelease" && campaign != "rogue" {
        Some("Tag and DeathBall require Ground Zero or Quake II rerelease".to_owned())
    } else {
        None
    }
}

/// Reason the selected map cannot host the rules, or `None` when playable.
///
/// Geometry is never guessed: source-specific objectives must be authored or
/// explicitly placed.
#[must_use]
pub fn match_map_unavailable(selection: &MatchModeSelection, classnames: &[String]) -> Option<String> {
    if let Some(reason) = match_mode_unavailable(selection) {
        return Some(reason);
    }
    if selection.rules == MatchRules::Deathball {
        let required = [
            "dm_dball_ball",
            "dm_dball_ball_start",
            "dm_dball_goal",
            "dm_dball_team1_start",
            "dm_dball_team2_start",
        ];
        let missing: Vec<&str> = required
            .iter()
            .copied()
            .filter(|name| !classnames.iter().any(|class| class == name))
            .collect();
        if !missing.is_empty() {
            return Some(format!("DeathBall map is missing: {}", missing.join(", ")));
        }
    }
    if selection.rules == MatchRules::Horde
        && (!classnames.iter().any(|class| class == "horde_manager")
            || !classnames.iter().any(|class| class.starts_with("info_monster_start")))
    {
        return Some("Horde requires an authored horde_manager and monster spawn points".to_owned());
    }
    if selection.rules == MatchRules::Ctf || selection.rules == MatchRules::Lmctf {
        let missing: Vec<&str> = ["item_flag_team1", "item_flag_team2"]
            .iter()
            .copied()
            .filter(|name| !classnames.iter().any(|class| class == name))
            .collect();
        if !missing.is_empty() {
            return Some(format!("CTF map is missing: {}", missing.join(", ")));
        }
    }
    if selection.mode == GameMode::Deathmatch
        && selection.rules != MatchRules::Deathball
        && !classnames.iter().any(|class| class == "info_player_deathmatch")
        && !(selection.family == GameFamily::Q3
            && classnames.iter().any(|class| class == "team_CTF_redplayer")
            && classnames.iter().any(|class| class == "team_CTF_blueplayer"))
    {
        return Some("Deathmatch requires an authored player spawn".to_owned());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection(family: GameFamily, rules: MatchRules) -> MatchModeSelection {
        MatchModeSelection {
            family,
            edition: "classic".to_owned(),
            campaign: "base".to_owned(),
            mode: GameMode::Deathmatch,
            rules,
        }
    }

    #[test]
    fn standard_rules_always_available() {
        let selected = selection(GameFamily::Q1, MatchRules::Standard);
        assert_eq!(match_mode_unavailable(&selected), None);
    }

    #[test]
    fn ctf_requires_q2_deathmatch_classic() {
        let mut selected = selection(GameFamily::Q1, MatchRules::Ctf);
        assert_eq!(
            match_mode_unavailable(&selected).as_deref(),
            Some("These match rules require a Quake II source game")
        );
        selected.family = GameFamily::Q2;
        selected.mode = GameMode::Coop;
        assert_eq!(
            match_mode_unavailable(&selected).as_deref(),
            Some("These match rules require deathmatch mode")
        );
        selected.mode = GameMode::Deathmatch;
        assert_eq!(match_mode_unavailable(&selected), None);
        selected.edition = "rerelease".to_owned();
        assert_eq!(
            match_mode_unavailable(&selected).as_deref(),
            Some("This CTF ruleset requires classic Quake II")
        );
    }

    #[test]
    fn horde_requires_rerelease_machine_or_past_without_deathmatch() {
        let mut selected = selection(GameFamily::Q1, MatchRules::Horde);
        selected.mode = GameMode::Coop;
        assert_eq!(
            match_mode_unavailable(&selected).as_deref(),
            Some("Horde requires Quake rerelease Dimension of the Machine or Dimension of the Past")
        );
        selected.edition = "rerelease".to_owned();
        selected.campaign = "mg1".to_owned();
        assert_eq!(match_mode_unavailable(&selected), None);
        selected.campaign = "dopa".to_owned();
        assert_eq!(match_mode_unavailable(&selected), None);
        selected.mode = GameMode::Deathmatch;
        assert_eq!(
            match_mode_unavailable(&selected).as_deref(),
            Some("Horde requires single player or cooperative mode")
        );
    }

    #[test]
    fn tag_requires_ground_zero_or_rerelease() {
        let mut selected = selection(GameFamily::Q2, MatchRules::Tag);
        assert_eq!(
            match_mode_unavailable(&selected).as_deref(),
            Some("Tag and DeathBall require Ground Zero or Quake II rerelease")
        );
        selected.campaign = "rogue".to_owned();
        assert_eq!(match_mode_unavailable(&selected), None);
    }

    #[test]
    fn deathball_map_reports_missing_objectives() {
        let mut selected = selection(GameFamily::Q2, MatchRules::Deathball);
        selected.campaign = "rogue".to_owned();
        let reason = match_map_unavailable(&selected, &[]).expect("missing objectives");
        assert!(reason.starts_with("DeathBall map is missing: "));
        assert!(reason.contains("dm_dball_goal"));
    }

    #[test]
    fn horde_map_requires_manager_and_spawns() {
        let mut selected = selection(GameFamily::Q1, MatchRules::Horde);
        selected.edition = "rerelease".to_owned();
        selected.campaign = "mg1".to_owned();
        selected.mode = GameMode::Coop;
        let reason = match_map_unavailable(&selected, &[]).expect("missing horde setup");
        assert_eq!(
            reason,
            "Horde requires an authored horde_manager and monster spawn points"
        );
        let classes = ["horde_manager".to_owned(), "info_monster_start_1".to_owned()];
        assert_eq!(match_map_unavailable(&selected, &classes), None);
    }

    #[test]
    fn deathmatch_requires_authored_spawn() {
        let selected = selection(GameFamily::Q2, MatchRules::Standard);
        assert_eq!(
            match_map_unavailable(&selected, &[]).as_deref(),
            Some("Deathmatch requires an authored player spawn")
        );
        let classes = ["info_player_deathmatch".to_owned()];
        assert_eq!(match_map_unavailable(&selected, &classes), None);
    }

    #[test]
    fn q3_ctf_spawns_satisfy_deathmatch() {
        let selected = selection(GameFamily::Q3, MatchRules::Standard);
        let classes = ["team_CTF_redplayer".to_owned(), "team_CTF_blueplayer".to_owned()];
        assert_eq!(match_map_unavailable(&selected, &classes), None);
    }
}
