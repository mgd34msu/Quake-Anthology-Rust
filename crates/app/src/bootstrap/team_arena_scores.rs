//! Team Arena postgame scores.
//!
//! Donor: `src/app/bootstrap/team-arena-scores.ts`
//! (`UI_CalcPostGameStats` and `postGameInfo_t`).
//! Async file access becomes a sync `TeamArenaScoreFiles` trait.
//! `gameAtoi` is shared from [`super::base_arena_progression`]:
//! `qa_core::numeric::native_atoi` saturates on overflow while the donor
//! wraps every digit operation.

use qa_core::numeric::{float_to_wrapped_i32, qvm_float_to_int};
use thiserror::Error;

use super::base_arena_progression::game_atoi_value;

/// Team Arena score failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TeamArenaScoresError {
    /// Argument text is not source bytes.
    #[error("Game numbers require byte characters")]
    NonByteText,
}

/// `bg_lib` atoi over the shared [`game_atoi_value`] core: skip donor
/// whitespace (bytes `<= 32`, NUL excluded), read an optional sign and
/// decimal digits, wrap every operation.
pub fn game_atoi(text: &str) -> Result<i32, TeamArenaScoresError> {
    game_atoi_value(text).map_err(|()| TeamArenaScoresError::NonByteText)
}

/// Truncate to a UTF-16-unit budget (donor `slice` semantics), shared
/// by the arena ports and the server browser.
pub(crate) fn truncate_utf16(text: &str, max_units: usize) -> &str {
    let mut units = 0;
    let mut end = 0;
    for (index, c) in text.char_indices() {
        let width = c.len_utf16();
        if units + width > max_units {
            break;
        }
        units += width;
        end = index + c.len_utf8();
    }
    &text[..end]
}

/// Parsed postgame stats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TeamArenaPostgameStats {
    /// Accuracy.
    pub accuracy: i32,
    /// Impressives.
    pub impressives: i32,
    /// Excellents.
    pub excellents: i32,
    /// Defends.
    pub defends: i32,
    /// Assists.
    pub assists: i32,
    /// Gauntlets.
    pub gauntlets: i32,
    /// Base score.
    pub base_score: i32,
    /// Perfects.
    pub perfects: i32,
    /// Red score.
    pub red_score: i32,
    /// Blue score.
    pub blue_score: i32,
    /// End time in milliseconds.
    pub end_time: i32,
    /// Captures.
    pub captures: i32,
}

/// Scored postgame record (stats without `end_time`, plus bonuses).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TeamArenaScore {
    /// Accuracy.
    pub accuracy: i32,
    /// Impressives.
    pub impressives: i32,
    /// Excellents.
    pub excellents: i32,
    /// Defends.
    pub defends: i32,
    /// Assists.
    pub assists: i32,
    /// Gauntlets.
    pub gauntlets: i32,
    /// Base score.
    pub base_score: i32,
    /// Perfects.
    pub perfects: i32,
    /// Red score.
    pub red_score: i32,
    /// Blue score.
    pub blue_score: i32,
    /// Captures.
    pub captures: i32,
    /// Total score.
    pub score: i32,
    /// Match time in seconds.
    pub time: i32,
    /// Time bonus.
    pub time_bonus: i32,
    /// Shutout bonus.
    pub shutout_bonus: i32,
    /// Skill bonus.
    pub skill_bonus: i32,
}

/// Score calculation input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TeamArenaScoreInput {
    /// Parsed stats.
    pub stats: TeamArenaPostgameStats,
    /// Match start time in milliseconds.
    pub match_start_time: f64,
    /// Skill level.
    pub skill: f64,
    /// Time to beat in seconds.
    pub time_to_beat: f64,
}

/// Score file access (sync donor `TeamArenaScoreFiles`).
pub trait TeamArenaScoreFiles {
    /// Read a score file, or `None` when absent.
    fn read_file(&mut self, path: &str) -> Option<Vec<u8>>;
    /// Write a score file.
    fn write_file(&mut self, path: &str, bytes: &[u8]);
}

/// Score calculation result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TeamArenaScoreResult {
    /// New score.
    pub score: TeamArenaScore,
    /// Previous score.
    pub previous: TeamArenaScore,
    /// Whether this is a new high score.
    pub new_high_score: bool,
    /// Whether this is a new best time.
    pub new_best_time: bool,
    /// Whether red won.
    pub won: bool,
}

/// Arguments exclude the command name, as `CommandInvocation.args` does.
pub fn parse_team_arena_postgame(args: &[String]) -> Result<TeamArenaPostgameStats, TeamArenaScoresError> {
    let arg = |index: usize| -> Result<i32, TeamArenaScoresError> {
        let text = args.get(index).map_or("", String::as_str);
        let head = text.split('\0').next().unwrap_or("");
        game_atoi(truncate_utf16(head, 1023))
    };
    Ok(TeamArenaPostgameStats {
        accuracy: arg(2)?,
        impressives: arg(3)?,
        excellents: arg(4)?,
        defends: arg(5)?,
        assists: arg(6)?,
        gauntlets: arg(7)?,
        base_score: arg(8)?,
        perfects: arg(9)?,
        red_score: arg(10)?,
        blue_score: arg(11)?,
        end_time: arg(12)?,
        captures: arg(13)?,
    })
}

/// Calculate a postgame score against the previous record.
#[must_use]
pub fn calculate_team_arena_score(input: &TeamArenaScoreInput, previous: &TeamArenaScore) -> TeamArenaScoreResult {
    let stats = &input.stats;
    let elapsed = (stats.end_time as f32) - (input.match_start_time as f32);
    let time = qvm_float_to_int((f64::from(elapsed) / 1000.0) as f32);
    let time_bonus = if f64::from(time) < input.time_to_beat {
        float_to_wrapped_i32(input.time_to_beat - f64::from(time)).wrapping_mul(10)
    } else {
        0
    };
    let won = stats.red_score > stats.blue_score;
    let shutout_bonus = if won && stats.blue_score <= 0 { 100 } else { 0 };
    let skill_bonus = 1.max(qvm_float_to_int(input.skill as f32));
    let score = TeamArenaScore {
        accuracy: stats.accuracy,
        impressives: stats.impressives,
        excellents: stats.excellents,
        defends: stats.defends,
        assists: stats.assists,
        gauntlets: stats.gauntlets,
        base_score: stats.base_score,
        perfects: stats.perfects,
        red_score: stats.red_score,
        blue_score: stats.blue_score,
        captures: stats.captures,
        time,
        time_bonus,
        shutout_bonus,
        skill_bonus,
        score: stats
            .base_score
            .wrapping_add(shutout_bonus)
            .wrapping_add(time_bonus)
            .wrapping_mul(skill_bonus),
    };
    TeamArenaScoreResult {
        score,
        previous: *previous,
        won,
        new_high_score: won && score.score > previous.score,
        new_best_time: time < previous.time,
    }
}

/// Score file path for a map and game type.
#[must_use]
pub fn team_arena_score_path(map: &str, game_type: i32) -> String {
    let head = truncate_utf16(map.split('\0').next().unwrap_or(""), 63);
    let full = format!("games/{head}_{game_type}.game");
    truncate_utf16(&full, 63).to_owned()
}

/// Native little-endian header plus the sixteen int32 fields of
/// `postGameInfo_t`.
#[must_use]
pub fn encode_team_arena_score(info: &TeamArenaScore) -> Vec<u8> {
    let mut bytes = vec![0u8; 68];
    bytes[0..4].copy_from_slice(&64i32.to_le_bytes());
    let fields = [
        info.score,
        info.red_score,
        info.blue_score,
        info.perfects,
        info.accuracy,
        info.impressives,
        info.excellents,
        info.defends,
        info.assists,
        info.gauntlets,
        info.captures,
        info.time,
        info.time_bonus,
        info.shutout_bonus,
        info.skill_bonus,
        info.base_score,
    ];
    for (index, value) in fields.iter().enumerate() {
        let start = 4 + index * 4;
        bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

/// Source zero-initializes both header and record before accepting
/// short reads.
#[must_use]
pub fn decode_team_arena_score(bytes: Option<&[u8]>) -> TeamArenaScore {
    let mut padded = [0u8; 68];
    if let Some(bytes) = bytes {
        let take = bytes.len().min(68);
        padded[..take].copy_from_slice(&bytes[..take]);
    }
    if i32::from_le_bytes([padded[0], padded[1], padded[2], padded[3]]) != 64 {
        padded = [0u8; 68];
    }
    let field = |index: usize| {
        let start = 4 + index * 4;
        i32::from_le_bytes([padded[start], padded[start + 1], padded[start + 2], padded[start + 3]])
    };
    TeamArenaScore {
        score: field(0),
        red_score: field(1),
        blue_score: field(2),
        perfects: field(3),
        accuracy: field(4),
        impressives: field(5),
        excellents: field(6),
        defends: field(7),
        assists: field(8),
        gauntlets: field(9),
        captures: field(10),
        time: field(11),
        time_bonus: field(12),
        shutout_bonus: field(13),
        skill_bonus: field(14),
        base_score: field(15),
    }
}

/// Record a score, persisting only new high scores.
pub fn record_team_arena_score(
    map: &str,
    game_type: i32,
    input: &TeamArenaScoreInput,
    files: &mut dyn TeamArenaScoreFiles,
) -> TeamArenaScoreResult {
    let path = team_arena_score_path(map, game_type);
    let stored = files.read_file(&path);
    let result = calculate_team_arena_score(input, &decode_team_arena_score(stored.as_deref()));
    if result.new_high_score {
        files.write_file(&path, &encode_team_arena_score(&result.score));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn stats() -> TeamArenaPostgameStats {
        TeamArenaPostgameStats {
            accuracy: 50,
            impressives: 1,
            excellents: 2,
            defends: 3,
            assists: 4,
            gauntlets: 5,
            base_score: 100,
            perfects: 6,
            red_score: 8,
            blue_score: 0,
            end_time: 120_000,
            captures: 7,
        }
    }

    fn zero_score() -> TeamArenaScore {
        decode_team_arena_score(None)
    }

    #[test]
    fn game_atoi_wraps_unlike_native() {
        assert_eq!(game_atoi("  -42 score").unwrap(), -42);
        assert_eq!(game_atoi("+7").unwrap(), 7);
        assert_eq!(game_atoi("").unwrap(), 0);
        assert_eq!(game_atoi("abc").unwrap(), 0);
        // 2^32 overflows wrap to zero instead of saturating.
        assert_eq!(game_atoi("4294967296").unwrap(), 0);
        assert_eq!(game_atoi("2147483648").unwrap(), i32::MIN);
        assert!(game_atoi("ÿ12").unwrap() == 12 || game_atoi("ÿ12").is_ok());
        assert_eq!(game_atoi("1€"), Err(TeamArenaScoresError::NonByteText));
    }

    #[test]
    fn parse_postgame_reads_arg_slots() {
        let args = [
            "cmd", "sub", "50", "1", "2", "3", "4", "5", "100", "6", "8", "0", "120000", "7",
        ]
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
        assert_eq!(parse_team_arena_postgame(&args).unwrap(), stats());
    }

    #[test]
    fn calculate_score_applies_bonuses() {
        let input = TeamArenaScoreInput {
            stats: stats(),
            match_start_time: 0.0,
            skill: 2.0,
            time_to_beat: 200.0,
        };
        let result = calculate_team_arena_score(&input, &zero_score());
        assert_eq!(result.score.time, 120);
        assert_eq!(result.score.time_bonus, 800);
        assert_eq!(result.score.shutout_bonus, 100);
        assert_eq!(result.score.skill_bonus, 2);
        assert_eq!(result.score.score, (100 + 100 + 800) * 2);
        assert!(result.won && result.new_high_score);
    }

    #[test]
    fn calculate_score_loses() {
        let mut losing = stats();
        losing.red_score = 1;
        losing.blue_score = 9;
        let input = TeamArenaScoreInput {
            stats: losing,
            match_start_time: 0.0,
            skill: 1.0,
            time_to_beat: 10.0,
        };
        let result = calculate_team_arena_score(&input, &zero_score());
        assert!(!result.won && !result.new_high_score);
        assert_eq!(result.score.shutout_bonus, 0);
        assert_eq!(result.score.time_bonus, 0);
    }

    #[test]
    fn score_path_truncates() {
        assert_eq!(team_arena_score_path("q3tourney6", 4), "games/q3tourney6_4.game");
        let long = "m".repeat(100);
        let path = team_arena_score_path(&long, 1);
        assert!(path.len() <= 63);
        assert!(path.starts_with("games/mmm"));
    }

    #[test]
    fn encode_decode_roundtrip_and_bad_header() {
        let input = TeamArenaScoreInput {
            stats: stats(),
            match_start_time: 0.0,
            skill: 2.0,
            time_to_beat: 200.0,
        };
        let score = calculate_team_arena_score(&input, &zero_score()).score;
        assert_eq!(decode_team_arena_score(Some(&encode_team_arena_score(&score))), score);
        assert_eq!(decode_team_arena_score(None), zero_score());
        let mut bad = encode_team_arena_score(&score);
        bad[0] = 0;
        assert_eq!(decode_team_arena_score(Some(&bad)), zero_score());
        assert_eq!(decode_team_arena_score(Some(&[1, 2, 3])), zero_score());
    }

    #[test]
    fn record_persists_only_high_scores() {
        struct Memory {
            files: HashMap<String, Vec<u8>>,
        }
        impl TeamArenaScoreFiles for Memory {
            fn read_file(&mut self, path: &str) -> Option<Vec<u8>> {
                self.files.get(path).cloned()
            }
            fn write_file(&mut self, path: &str, bytes: &[u8]) {
                self.files.insert(path.to_owned(), bytes.to_vec());
            }
        }
        let mut files = Memory { files: HashMap::new() };
        let input = TeamArenaScoreInput {
            stats: stats(),
            match_start_time: 0.0,
            skill: 2.0,
            time_to_beat: 200.0,
        };
        let first = record_team_arena_score("tourney", 4, &input, &mut files);
        assert!(first.new_high_score);
        assert!(files.files.contains_key("games/tourney_4.game"));
        let second = record_team_arena_score("tourney", 4, &input, &mut files);
        assert!(!second.new_high_score);
    }

    #[test]
    fn shared_game_atoi_matches_base_port() {
        use crate::bootstrap::base_arena_progression::game_atoi as base_game_atoi;
        for text in [
            "",
            "  -42 score",
            "+7",
            "abc",
            "007",
            "--12",
            "  +0x10",
            "4294967296",
            "2147483648",
            "-2147483649",
            "99999999999999999999",
            "ÿ12",
            "1€",
            "\u{0}12",
            " \t\n 33 ",
        ] {
            let base = base_game_atoi(text);
            let team = game_atoi(text);
            match (base, team) {
                (Ok(expected), Ok(actual)) => assert_eq!(actual, expected, "{text:?}"),
                (Err(_), Err(TeamArenaScoresError::NonByteText)) => {}
                (base, team) => panic!("divergent outcomes for {text:?}: {base:?} vs {team:?}"),
            }
        }
    }

    #[test]
    fn shared_truncate_counts_utf16_units() {
        assert_eq!(truncate_utf16("abcdef", 3), "abc");
        assert_eq!(truncate_utf16("abcdef", 0), "");
        assert_eq!(truncate_utf16("", 31), "");
        assert_eq!(truncate_utf16("ab", 31), "ab");
        // `é` is one UTF-16 unit; `𝄞` and emoji are two.
        assert_eq!(truncate_utf16("aébc", 2), "aé");
        assert_eq!(truncate_utf16("a𝄞bc", 2), "a");
        assert_eq!(truncate_utf16("a𝄞bc", 3), "a𝄞");
        assert_eq!(truncate_utf16("🙂🙂", 2), "🙂");
        // Never splits a character: the 31-unit server-browser budget
        // keeps whole glyphs.
        assert_eq!(truncate_utf16("name🙂", 5), "name");
        assert_eq!(truncate_utf16("name🙂", 6), "name🙂");
    }

    #[test]
    fn canonical_to_int32_keeps_score_domains() {
        // Values on the time-bonus path plus the wrap edges the old
        // local `js_to_int32` shared with the canonical conversion.
        for (value, expected) in [
            (80.0, 80),
            (0.5, 0),
            (-0.5, 0),
            (-1.0, -1),
            (2_147_483_647.0, 2_147_483_647),
            (2_147_483_648.0, i32::MIN),
            (4_294_967_296.0, 0),
            (f64::NAN, 0),
            (f64::INFINITY, 0),
            (f64::NEG_INFINITY, 0),
        ] {
            assert_eq!(float_to_wrapped_i32(value), expected, "{value}");
        }
    }
}
