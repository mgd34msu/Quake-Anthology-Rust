//! Q2 mission-pack modes barrel (`src/content/q2/missionpacks/modes/index.ts`).
//!
//! Pure re-export barrel: tag and deathball live in the sibling modules
//! below.
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

pub mod deathball;
pub mod tag;

pub use deathball::{q2_deathball_rules, Q2DeathBall, Q2DeathBallCheckpoint, Q2DeathBallHooks};
pub use tag::{Q2Tag, Q2TagCheckpoint, Q2TagHooks};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deathball_rules_force_mode_flags() {
        let rules = q2_deathball_rules(0);
        assert_eq!(rules.stop_speed, 0.0);
        assert_eq!(
            rules.deathmatch_flags & (0x20000 | 0x80000 | 0x40000 | 256 | 64),
            0x20000 | 0x80000 | 0x40000 | 256 | 64
        );
        let preserved = q2_deathball_rules(1);
        assert_eq!(preserved.deathmatch_flags & 1, 1);
    }

    #[test]
    fn mode_checkpoints_default_to_empty_match() {
        let tag = Q2TagCheckpoint {
            token: None,
            owner: None,
            count: 0,
        };
        assert_eq!(tag.count, 0);
        let ball = Q2DeathBallCheckpoint {
            ball: None,
            starts: 0,
            team1_score: 0.0,
            team2_score: 0.0,
        };
        assert_eq!(ball.team1_score, ball.team2_score);
    }

    #[test]
    fn mode_callback_tables_cover_source_names() {
        let tag = tag::tag_callbacks();
        assert!(tag.think.contains_key("Tag_Respawn"));
        assert!(tag.touch.contains_key("Tag_TouchItem"));
        let ball = deathball::deathball_callbacks();
        assert!(ball.think.contains_key("DBall_BallRespawn"));
        assert!(ball.touch.contains_key("DBall_BallTouch"));
        assert!(ball.die.contains_key("DBall_BallDie"));
    }
}
