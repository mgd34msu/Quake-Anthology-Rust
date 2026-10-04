//! Base single-player arena progression.
//!
//! Donor: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/base-arena-progression.ts`
//! (`ArenaSkill`, `BaseArenaProgression`, score/award/video cvars).
//! Scores live in `g_spScores<skill>` info strings, awards in `g_spAwards`,
//! unlocked movies in `g_spVideos`, following the donor's `ui_gameinfo.c`
//! and `ui_sppostgame.c` semantics.

use qa_compat::userinfo::info_value_for_key;
use qa_compat::CompatError;
use qa_core::cmd::Dialect;
use qa_core::cvar::{flags, set_info_value, CvarError, CvarRegistry, InfoOptions, InfoTarget};
use thiserror::Error;

/// Base arena progression failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BaseArenaProgressionError {
    /// Catalog levels are negative, unaligned, or inverted.
    #[error("Invalid arena progression catalog")]
    InvalidCatalog,
    /// Recorded result level or rank is out of range.
    #[error("Invalid arena result")]
    InvalidResult,
    /// Score text is not source bytes.
    #[error("Game numbers require byte characters")]
    NonByteText,
    /// Info-string lookup failure.
    #[error(transparent)]
    Info(#[from] CompatError),
    /// Info-string write rejection (the donor throws the print text).
    #[error("{0}")]
    InfoWrite(String),
    /// Cvar registry failure.
    #[error(transparent)]
    Cvar(#[from] CvarError),
}

/// `bg_lib` atoi (`gameAtoi`) core shared by the arena ports:
/// skip donor whitespace (signed bytes `<= 32`, NUL excluded), read an
/// optional sign and decimal digits, wrap every operation.
/// `qa_core::numeric::native_atoi` saturates on overflow while the donor
/// wraps, so the ports keep this local core. `Err(())` means the text is
/// not source bytes.
pub(crate) fn game_atoi_value(text: &str) -> Result<i32, ()> {
    let mut bytes = Vec::with_capacity(text.len());
    for c in text.chars() {
        if c as u32 > 255 {
            return Err(());
        }
        bytes.push(c as u8);
    }
    let mut offset = 0;
    while offset < bytes.len() && bytes[offset] != 0 && (bytes[offset] as i8 as i32) <= 32 {
        offset += 1;
    }
    if offset >= bytes.len() || bytes[offset] == 0 {
        return Ok(0);
    }
    let mut sign = 1i32;
    if bytes[offset] == b'+' || bytes[offset] == b'-' {
        sign = if bytes[offset] == b'-' { -1 } else { 1 };
        offset += 1;
    }
    let mut value = 0i32;
    while offset < bytes.len() {
        let byte = bytes[offset];
        if !byte.is_ascii_digit() {
            break;
        }
        value = value.wrapping_mul(10).wrapping_add(i32::from(byte - b'0'));
        offset += 1;
    }
    Ok(value.wrapping_mul(sign))
}

/// `bg_lib` atoi (`gameAtoi`) over the shared [`game_atoi_value`] core.
pub fn game_atoi(text: &str) -> Result<i32, BaseArenaProgressionError> {
    game_atoi_value(text).map_err(|()| BaseArenaProgressionError::NonByteText)
}

/// Arena difficulty (`ArenaSkill`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArenaSkill {
    /// `I Can Win`.
    One = 1,
    /// `Bring It On`.
    Two = 2,
    /// `Hurt Me Plenty`.
    Three = 3,
    /// `Hardcore`.
    Four = 4,
    /// `Nightmare`.
    Five = 5,
}

impl ArenaSkill {
    /// Skill number (`1..=5`).
    #[must_use]
    pub fn value(self) -> i32 {
        self as i32
    }

    /// Parse a skill number.
    pub fn from_value(value: i32) -> Result<Self, BaseArenaProgressionError> {
        match value {
            1 => Ok(Self::One),
            2 => Ok(Self::Two),
            3 => Ok(Self::Three),
            4 => Ok(Self::Four),
            5 => Ok(Self::Five),
            _ => Err(BaseArenaProgressionError::InvalidResult),
        }
    }
}

/// Level layout backing progression (`ArenaProgressionCatalog`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArenaProgressionCatalog {
    /// Regular level count (multiple of four).
    pub regular_levels: i32,
    /// Training level, if any.
    pub training: Option<i32>,
    /// Final level, if any.
    pub final_level: Option<i32>,
    /// Total level count including specials.
    pub total_levels: i32,
}

/// One completed match (`ArenaResult`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArenaResult {
    /// Level number.
    pub level: i32,
    /// Played skill.
    pub skill: ArenaSkill,
    /// Finish rank (`1..=8`).
    pub rank: i32,
    /// Accuracy percentage.
    pub accuracy: i32,
    /// Impressive count.
    pub impressive: i32,
    /// Excellent count.
    pub excellent: i32,
    /// Gauntlet count.
    pub gauntlet: i32,
    /// Frag count.
    pub frags: i32,
    /// Flawless victory.
    pub perfect: bool,
}

/// One granted medal (`ArenaAward`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArenaAward {
    /// Medal index.
    pub medal: i32,
    /// Display amount.
    pub amount: i32,
}

/// Best rank and the skill that earned it (`best`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArenaBest {
    /// Best rank (`0` means uncompleted).
    pub rank: i32,
    /// Skill of the best rank (`0` means uncompleted).
    pub skill: i32,
}

/// Recorded-match outcome (`ArenaProgressionResult`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArenaProgressionResult {
    /// Recorded rank.
    pub rank: i32,
    /// Completed tier (`-1` means none).
    pub completed_tier: i32,
    /// Newly unlocked movie tier, if any.
    pub unlocked_movie: Option<i32>,
    /// Medals earned by this match.
    pub awards: Vec<ArenaAward>,
    /// Next playable level (`-1` means everything is done).
    pub next_level: i32,
}

/// Base arena progression over archived score cvars (`BaseArenaProgression`).
pub struct BaseArenaProgression {
    cvars: CvarRegistry,
    catalog: ArenaProgressionCatalog,
}

impl std::fmt::Debug for BaseArenaProgression {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BaseArenaProgression")
            .field("catalog", &self.catalog)
            .finish_non_exhaustive()
    }
}

impl BaseArenaProgression {
    /// Register the score, award, and video cvars for a catalog.
    pub fn new(mut cvars: CvarRegistry, catalog: ArenaProgressionCatalog) -> Result<Self, BaseArenaProgressionError> {
        if catalog.regular_levels < 0
            || catalog.regular_levels % 4 != 0
            || catalog.total_levels < catalog.regular_levels
        {
            return Err(BaseArenaProgressionError::InvalidCatalog);
        }
        for skill in 1..=5 {
            cvars.register(&format!("g_spScores{skill}"), "", flags::ARCHIVE)?;
        }
        cvars.register("g_spAwards", "", flags::ARCHIVE)?;
        cvars.register("g_spVideos", "", flags::ARCHIVE)?;
        Ok(Self { cvars, catalog })
    }

    /// Borrow the backing registry.
    #[must_use]
    pub fn cvars(&self) -> &CvarRegistry {
        &self.cvars
    }

    /// Mutably borrow the backing registry.
    pub fn cvars_mut(&mut self) -> &mut CvarRegistry {
        &mut self.cvars
    }

    /// Borrow the level layout.
    #[must_use]
    pub fn catalog(&self) -> ArenaProgressionCatalog {
        self.catalog
    }

    fn read(&self, name: &str, key: &str) -> Result<i32, BaseArenaProgressionError> {
        let current = self.cvars.get(name).map_or(String::new(), |entry| entry.value);
        game_atoi(&info_value_for_key(&current, key, 8192)?)
    }

    fn write(&mut self, name: &str, key: &str, value: i32) -> Result<(), BaseArenaProgressionError> {
        let current = self.cvars.get(name).map_or(String::new(), |entry| entry.value);
        let mut rejection: Option<String> = None;
        let text = set_info_value(
            &current,
            key,
            &value.to_string(),
            InfoOptions {
                dialect: Dialect::Q3,
                maximum_length: 1024,
                target: InfoTarget::ClientUserinfo,
                server_high_characters: true,
            },
            &mut |message: &str| {
                rejection = Some(message.to_string());
            },
        )?;
        if let Some(message) = rejection {
            return Err(BaseArenaProgressionError::InfoWrite(message));
        }
        self.cvars.set(name, &text, true)?;
        Ok(())
    }

    /// Best rank across skills for one level (`best`).
    pub fn best(&self, level: i32) -> Result<ArenaBest, BaseArenaProgressionError> {
        let mut best = ArenaBest { rank: 0, skill: 0 };
        for skill in 1..=5 {
            let current = self.read(&format!("g_spScores{skill}"), &format!("l{level}"))?;
            if (1..=8).contains(&current) && (best.rank == 0 || current <= best.rank) {
                best = ArenaBest { rank: current, skill };
            }
        }
        Ok(best)
    }

    /// Stored award count for one medal (`award`).
    pub fn award(&self, medal: i32) -> Result<i32, BaseArenaProgressionError> {
        self.read("g_spAwards", &format!("a{medal}"))
    }

    /// Whether a movie tier is unlocked (`movieUnlocked`).
    pub fn movie_unlocked(&self, tier: i32) -> Result<bool, BaseArenaProgressionError> {
        Ok(tier > 0 && self.read("g_spVideos", &format!("tier{tier}"))? != 0)
    }

    /// First unbeaten level, the final level, or `-1` (`currentLevel`).
    pub fn current_level(&self) -> Result<i32, BaseArenaProgressionError> {
        if let Some(training) = self.catalog.training {
            if self.best(training)?.rank != 1 {
                return Ok(training);
            }
        }
        for level in 0..self.catalog.regular_levels {
            if self.best(level)?.rank != 1 {
                return Ok(level);
            }
        }
        Ok(self.catalog.final_level.unwrap_or(-1))
    }

    /// Whether a level is selectable (`levelAvailable`).
    pub fn level_available(&self, level: i32) -> Result<bool, BaseArenaProgressionError> {
        if Some(level) == self.catalog.training {
            return Ok(true);
        }
        if level < 0 || level >= self.catalog.total_levels {
            return Ok(false);
        }
        let current = self.current_level()?;
        if Some(current) == self.catalog.training {
            return Ok(false);
        }
        if current < 0 {
            return Ok(true);
        }
        let current_tier = if Some(current) == self.catalog.final_level {
            self.catalog.regular_levels / 4
        } else {
            current.div_euclid(4)
        };
        let tier = if Some(level) == self.catalog.final_level {
            self.catalog.regular_levels / 4
        } else {
            level.div_euclid(4)
        };
        Ok(tier <= current_tier)
    }

    /// Completed tier for a level (`completedTier`).
    pub fn completed_tier(&self, level: i32) -> Result<i32, BaseArenaProgressionError> {
        if Some(level) == self.catalog.training {
            return Ok(0);
        }
        if Some(level) == self.catalog.final_level {
            return Ok(self.catalog.regular_levels / 4 + 1);
        }
        if level < 0 || level >= self.catalog.regular_levels {
            return Ok(-1);
        }
        let tier = level.div_euclid(4);
        for member in tier * 4..tier * 4 + 4 {
            if self.best(member)?.rank != 1 {
                return Ok(-1);
            }
        }
        Ok(tier + 1)
    }

    /// Record one match: best rank, medals, and movie unlock (`record`).
    pub fn record(&mut self, result: &ArenaResult) -> Result<ArenaProgressionResult, BaseArenaProgressionError> {
        if result.level < 0 || result.level >= self.catalog.total_levels || !(1..=8).contains(&result.rank) {
            return Err(BaseArenaProgressionError::InvalidResult);
        }
        let name = format!("g_spScores{}", result.skill.value());
        let key = format!("l{}", result.level);
        let old = self.read(&name, &key)?;
        if old == 0 || old > result.rank {
            self.write(&name, &key, result.rank)?;
        }
        let mut awards = Vec::new();
        let grants = [
            (0, i32::from(result.accuracy >= 50), result.accuracy),
            (1, result.impressive, result.impressive),
            (2, result.excellent, result.excellent),
            (3, result.gauntlet, result.gauntlet),
        ];
        for (medal, count, display) in grants {
            if count == 0 {
                continue;
            }
            let total = self.award(medal)?.wrapping_add(count);
            self.write("g_spAwards", &format!("a{medal}"), total)?;
            if display != 0 {
                awards.push(ArenaAward { medal, amount: display });
            }
        }
        let old_hundreds = self.award(4)? / 100;
        if result.frags != 0 {
            let total = self.award(4)?.wrapping_add(result.frags);
            self.write("g_spAwards", "a4", total)?;
        }
        let hundreds = self.award(4)? / 100;
        if hundreds > old_hundreds {
            awards.push(ArenaAward {
                medal: 4,
                amount: hundreds.wrapping_mul(100),
            });
        }
        if result.perfect {
            let total = self.award(5)?.wrapping_add(1);
            self.write("g_spAwards", "a5", total)?;
            awards.push(ArenaAward { medal: 5, amount: 1 });
        }
        let tier = if result.rank == 1 {
            self.completed_tier(result.level)?
        } else {
            -1
        };
        let unlocked_movie = if tier >= 0 && !self.movie_unlocked(tier + 1)? {
            Some(tier + 1)
        } else {
            None
        };
        if let Some(movie) = unlocked_movie {
            self.write("g_spVideos", &format!("tier{movie}"), 1)?;
        }
        Ok(ArenaProgressionResult {
            rank: result.rank,
            completed_tier: tier,
            unlocked_movie,
            awards,
            next_level: self.current_level()?,
        })
    }

    /// Clear every score, award, and video (`reset`).
    pub fn reset(&mut self) -> Result<(), BaseArenaProgressionError> {
        for skill in 1..=5 {
            self.cvars.set(&format!("g_spScores{skill}"), "", true)?;
        }
        self.cvars.set("g_spAwards", "", true)?;
        self.cvars.set("g_spVideos", "", true)?;
        Ok(())
    }

    /// Mark every level beaten and every movie unlocked (`unlockLevels`).
    pub fn unlock_levels(&mut self) -> Result<(), BaseArenaProgressionError> {
        for level in 0..self.catalog.total_levels {
            self.write("g_spScores1", &format!("l{level}"), 1)?;
        }
        for tier in 1..=8 {
            self.write("g_spVideos", &format!("tier{tier}"), 1)?;
        }
        Ok(())
    }

    /// Grant one hundred of every medal (`unlockMedals`).
    pub fn unlock_medals(&mut self) -> Result<(), BaseArenaProgressionError> {
        for medal in 0..6 {
            self.write("g_spAwards", &format!("a{medal}"), 100)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> ArenaProgressionCatalog {
        ArenaProgressionCatalog {
            regular_levels: 8,
            training: Some(8),
            final_level: Some(9),
            total_levels: 10,
        }
    }

    fn progression() -> BaseArenaProgression {
        BaseArenaProgression::new(CvarRegistry::new(Dialect::Q3), catalog()).unwrap()
    }

    fn result(level: i32, rank: i32) -> ArenaResult {
        ArenaResult {
            level,
            skill: ArenaSkill::Three,
            rank,
            accuracy: 0,
            impressive: 0,
            excellent: 0,
            gauntlet: 0,
            frags: 0,
            perfect: false,
        }
    }

    #[test]
    fn atoi_matches_donor_wrapping_profile() {
        assert_eq!(game_atoi("  -42x"), Ok(-42));
        assert_eq!(game_atoi("+17"), Ok(17));
        assert_eq!(game_atoi(""), Ok(0));
        assert_eq!(game_atoi("9999999999"), Ok(9_999_999_999_i64 as i32));
        assert!(game_atoi("\u{20ac}").is_err());
    }

    #[test]
    fn shared_core_maps_non_byte_text() {
        assert_eq!(game_atoi_value("12"), Ok(12));
        assert_eq!(game_atoi_value("1€"), Err(()));
        assert_eq!(game_atoi("1€"), Err(BaseArenaProgressionError::NonByteText));
    }

    #[test]
    fn rejects_invalid_catalogs() {
        let bad = ArenaProgressionCatalog {
            regular_levels: 6,
            ..catalog()
        };
        assert_eq!(
            BaseArenaProgression::new(CvarRegistry::new(Dialect::Q3), bad).unwrap_err(),
            BaseArenaProgressionError::InvalidCatalog
        );
        let bad = ArenaProgressionCatalog {
            total_levels: 4,
            ..catalog()
        };
        assert!(BaseArenaProgression::new(CvarRegistry::new(Dialect::Q3), bad).is_err());
    }

    #[test]
    fn training_gates_regular_levels() {
        let mut progress = progression();
        assert_eq!(progress.current_level().unwrap(), 8);
        assert!(progress.level_available(8).unwrap());
        assert!(!progress.level_available(0).unwrap());
        assert!(!progress.level_available(-1).unwrap());
        assert!(!progress.level_available(10).unwrap());
        let outcome = progress.record(&result(8, 1)).unwrap();
        assert_eq!(outcome.completed_tier, 0);
        assert_eq!(outcome.unlocked_movie, Some(1));
        assert!(progress.movie_unlocked(1).unwrap());
        assert_eq!(progress.current_level().unwrap(), 0);
        assert!(progress.level_available(0).unwrap());
        assert!(progress.level_available(3).unwrap());
        assert!(!progress.level_available(4).unwrap());
        assert!(!progress.level_available(9).unwrap());
    }

    #[test]
    fn best_rank_tracks_lowest_across_skills() {
        let mut progress = progression();
        progress.record(&result(8, 1)).unwrap();
        let mut skilled = result(0, 3);
        skilled.skill = ArenaSkill::One;
        progress.record(&skilled).unwrap();
        assert_eq!(progress.best(0).unwrap(), ArenaBest { rank: 3, skill: 1 });
        progress.record(&result(0, 2)).unwrap();
        assert_eq!(progress.best(0).unwrap(), ArenaBest { rank: 2, skill: 3 });
        // A worse rank never overwrites the stored best.
        progress.record(&result(0, 5)).unwrap();
        assert_eq!(progress.best(0).unwrap(), ArenaBest { rank: 2, skill: 3 });
    }

    #[test]
    fn tier_completion_unlocks_movies() {
        let mut progress = progression();
        progress.record(&result(8, 1)).unwrap();
        for level in 0..4 {
            let outcome = progress.record(&result(level, 1)).unwrap();
            if level < 3 {
                assert_eq!(outcome.completed_tier, -1);
                assert_eq!(outcome.unlocked_movie, None);
            } else {
                assert_eq!(outcome.completed_tier, 1);
                assert_eq!(outcome.unlocked_movie, Some(2));
            }
        }
        assert_eq!(progress.current_level().unwrap(), 4);
        assert!(progress.level_available(7).unwrap());
        // Off-rank finishes never complete a tier.
        let outcome = progress.record(&result(4, 2)).unwrap();
        assert_eq!(outcome.completed_tier, -1);
    }

    #[test]
    fn grants_medals_and_hundred_frag_milestones() {
        let mut progress = progression();
        progress.record(&result(8, 1)).unwrap();
        let scored = ArenaResult {
            accuracy: 60,
            impressive: 2,
            excellent: 1,
            gauntlet: 3,
            frags: 120,
            perfect: true,
            ..result(0, 1)
        };
        let outcome = progress.record(&scored).unwrap();
        assert!(outcome.awards.contains(&ArenaAward { medal: 0, amount: 60 }));
        assert!(outcome.awards.contains(&ArenaAward { medal: 1, amount: 2 }));
        assert!(outcome.awards.contains(&ArenaAward { medal: 4, amount: 100 }));
        assert!(outcome.awards.contains(&ArenaAward { medal: 5, amount: 1 }));
        assert_eq!(progress.award(4).unwrap(), 120);
        // Low accuracy earns no medal.
        let outcome = progress.record(&result(1, 1)).unwrap();
        assert!(!outcome.awards.iter().any(|award| award.medal == 0));
    }

    #[test]
    fn rejects_invalid_results() {
        let mut progress = progression();
        assert_eq!(
            progress.record(&result(10, 1)).unwrap_err(),
            BaseArenaProgressionError::InvalidResult
        );
        assert_eq!(
            progress.record(&result(0, 0)).unwrap_err(),
            BaseArenaProgressionError::InvalidResult
        );
        assert_eq!(
            progress.record(&result(0, 9)).unwrap_err(),
            BaseArenaProgressionError::InvalidResult
        );
    }

    #[test]
    fn reset_and_unlocks_cover_every_slot() {
        let mut progress = progression();
        progress.unlock_levels().unwrap();
        progress.unlock_medals().unwrap();
        // The donor returns the final level once everything regular is
        // beaten, even when the final itself is beaten.
        assert_eq!(progress.current_level().unwrap(), 9);
        assert!(progress.level_available(9).unwrap());
        assert!(progress.level_available(0).unwrap());
        assert!(progress.movie_unlocked(8).unwrap());
        assert_eq!(progress.award(0).unwrap(), 100);
        progress.reset().unwrap();
        assert_eq!(progress.best(0).unwrap(), ArenaBest { rank: 0, skill: 0 });
        assert_eq!(progress.award(0).unwrap(), 0);
        assert!(!progress.movie_unlocked(1).unwrap());
        assert_eq!(progress.current_level().unwrap(), 8);
    }
}
