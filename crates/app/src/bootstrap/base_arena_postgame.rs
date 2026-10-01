//! Base single-player arena postgame parsing and presentation.
//!
//! Donor: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/base-arena-postgame.ts`
//! (`parseArenaPostgame`, `arenaPostgamePresentation`).
//! `gameAtoi` is shared from [`super::base_arena_progression`].

use thiserror::Error;

use super::base_arena_progression::{
    game_atoi, ArenaProgressionResult, ArenaResult, ArenaSkill, BaseArenaProgressionError,
};

/// Base arena postgame failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BaseArenaPostgameError {
    /// Postgame argument text is not source bytes.
    #[error(transparent)]
    Scores(#[from] BaseArenaProgressionError),
}

/// One podium row (`ArenaPodiumPlayer`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArenaPodiumPlayer {
    /// Client number.
    pub client: i32,
    /// One-based rank.
    pub rank: i32,
    /// Score.
    pub score: i32,
}

/// Parsed postgame (`ArenaPostgame`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArenaPostgame {
    /// Match result.
    pub result: ArenaResult,
    /// Podium players.
    pub players: Vec<ArenaPodiumPlayer>,
    /// Viewing player client.
    pub player_client: i32,
}

/// Postgame music (`musicCommand`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PostgameMusic {
    /// Victory music.
    Win,
    /// Defeat music.
    Loss,
}

impl PostgameMusic {
    /// Console command playing the music.
    #[must_use]
    pub fn command(self) -> &'static str {
        match self {
            Self::Win => "music music/win",
            Self::Loss => "music music/loss",
        }
    }
}

/// Postgame controls (`controls`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PostgameControl {
    /// Retry the match.
    Retry,
    /// Advance to the next match.
    Next,
    /// Return to the main menu.
    Main,
}

impl PostgameControl {
    /// Control identity.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Retry => "retry",
            Self::Next => "next",
            Self::Main => "main",
        }
    }
}

/// Winner announcement delay in milliseconds.
pub const WINNER_ANNOUNCEMENT_AFTER_MS: u64 = 1000;

/// Postgame presentation (`ArenaPostgamePresentation`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArenaPostgamePresentation {
    /// Recorded-match outcome.
    pub result: ArenaProgressionResult,
    /// Top three podium rows.
    pub podium: Vec<ArenaPodiumPlayer>,
    /// Victory or defeat music.
    pub music: PostgameMusic,
    /// Winner announcement delay; always [`WINNER_ANNOUNCEMENT_AFTER_MS`].
    pub winner_announcement_after_ms: u64,
    /// Postgame controls.
    pub controls: [PostgameControl; 3],
}

/// Parse native `spPostgame` arguments (`parseArenaPostgame`).
///
/// Arguments exclude `spPostgame`; the native game sends zero-based ranks
/// with the `RANK_TIED` bit 64 set.
pub fn parse_arena_postgame(
    args: &[&str],
    level: i32,
    skill: ArenaSkill,
) -> Result<ArenaPostgame, BaseArenaPostgameError> {
    let number = |index: usize| -> Result<i32, BaseArenaPostgameError> {
        let text = args.get(index).copied().unwrap_or("");
        let head = text.split('\0').next().unwrap_or("");
        let clipped: String = head.chars().take(1023).collect();
        Ok(game_atoi(&clipped)?)
    };
    let count = number(0)?.clamp(0, 8);
    let player_client = number(1)?;
    let mut players = Vec::new();
    let mut rank = 8;
    for n in 0..count {
        let base = 8 + (n as usize) * 3;
        let client = number(base)?;
        let player_rank = (number(base + 1)? & !64).wrapping_add(1);
        players.push(ArenaPodiumPlayer {
            client,
            rank: player_rank,
            score: number(base + 2)?,
        });
        if client == player_client {
            rank = player_rank;
        }
    }
    Ok(ArenaPostgame {
        player_client,
        players,
        result: ArenaResult {
            level,
            skill,
            rank,
            accuracy: number(2)?,
            impressive: number(3)?,
            excellent: number(4)?,
            gauntlet: number(5)?,
            frags: number(6)?,
            perfect: number(7)? != 0,
        },
    })
}

/// Build the postgame presentation (`arenaPostgamePresentation`).
#[must_use]
pub fn arena_postgame_presentation(game: &ArenaPostgame, result: ArenaProgressionResult) -> ArenaPostgamePresentation {
    ArenaPostgamePresentation {
        music: if result.rank == 1 {
            PostgameMusic::Win
        } else {
            PostgameMusic::Loss
        },
        podium: game.players.iter().take(3).copied().collect(),
        result,
        winner_announcement_after_ms: WINNER_ANNOUNCEMENT_AFTER_MS,
        controls: [PostgameControl::Retry, PostgameControl::Next, PostgameControl::Main],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bootstrap::base_arena_progression::ArenaAward;

    fn recorded(rank: i32) -> ArenaProgressionResult {
        ArenaProgressionResult {
            rank,
            completed_tier: 1,
            unlocked_movie: Some(2),
            awards: vec![ArenaAward { medal: 0, amount: 60 }],
            next_level: 1,
        }
    }

    fn args() -> Vec<String> {
        // count, player client, accuracy, impressive, excellent, gauntlet,
        // frags, perfect, then (client, rank, score) triples.
        vec![
            "2", "3", "55", "1", "2", "0", "12", "1", "3", "0", "20", "5", "65", "18",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    }

    #[test]
    fn parses_native_postgame_arguments() {
        let owned = args();
        let borrowed: Vec<&str> = owned.iter().map(String::as_str).collect();
        let game = parse_arena_postgame(&borrowed, 4, ArenaSkill::Two).unwrap();
        assert_eq!(game.player_client, 3);
        assert_eq!(
            game.players,
            vec![
                ArenaPodiumPlayer {
                    client: 3,
                    rank: 1,
                    score: 20
                },
                ArenaPodiumPlayer {
                    client: 5,
                    rank: 2,
                    score: 18
                },
            ]
        );
        assert_eq!(
            game.result,
            ArenaResult {
                level: 4,
                skill: ArenaSkill::Two,
                rank: 1,
                accuracy: 55,
                impressive: 1,
                excellent: 2,
                gauntlet: 0,
                frags: 12,
                perfect: true,
            }
        );
    }

    #[test]
    fn missing_player_keeps_default_rank_and_clips_nul() {
        let owned = ["1".to_string(), "9".to_string(), "0\0ignored".to_string()];
        let borrowed: Vec<&str> = owned.iter().map(String::as_str).collect();
        let game = parse_arena_postgame(&borrowed, 0, ArenaSkill::One).unwrap();
        assert_eq!(game.result.rank, 8);
        assert_eq!(game.result.accuracy, 0);
        assert_eq!(game.players.len(), 1);
    }

    #[test]
    fn count_clamps_to_eight_players() {
        let mut owned = vec!["99".to_string(), "0".to_string()];
        owned.extend((0..8).flat_map(|n| vec![n.to_string(), "0".to_string(), "10".to_string()]));
        let borrowed: Vec<&str> = owned.iter().map(String::as_str).collect();
        let game = parse_arena_postgame(&borrowed, 0, ArenaSkill::One).unwrap();
        assert_eq!(game.players.len(), 8);
    }

    #[test]
    fn presentation_keeps_podium_and_music() {
        let owned = args();
        let borrowed: Vec<&str> = owned.iter().map(String::as_str).collect();
        let game = parse_arena_postgame(&borrowed, 0, ArenaSkill::One).unwrap();
        let win = arena_postgame_presentation(&game, recorded(1));
        assert_eq!(win.music, PostgameMusic::Win);
        assert_eq!(win.music.command(), "music music/win");
        assert_eq!(win.podium.len(), 2);
        assert_eq!(win.winner_announcement_after_ms, 1000);
        assert_eq!(win.controls.map(PostgameControl::id), ["retry", "next", "main"]);
        let loss = arena_postgame_presentation(&game, recorded(4));
        assert_eq!(loss.music.command(), "music music/loss");
    }
}
