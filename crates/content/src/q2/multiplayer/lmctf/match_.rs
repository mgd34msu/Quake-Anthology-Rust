//! Q2 LMCTF match rules (`src/content/q2/multiplayer/lmctf/match.ts`).
//!
//! LM_CTF 6.0 g_tourney.c. GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::q2::foundation::host::{Q2GameServices, Q2PresentationEvent};

use super::types::{lmctf_print, LmctfHooks, LmctfMapChange};

/// LMCTF match phase (`LmctfMatch["phase"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LmctfMatchPhase {
    /// No match.
    #[default]
    None,
    /// Countdown.
    Countdown,
    /// In play.
    Inplay,
    /// Over.
    Over,
}

/// LMCTF match state.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LmctfMatchState {
    /// Phase.
    pub phase: LmctfMatchPhase,
    /// Pending map.
    pub pending_map: Option<LmctfMapChange>,
    /// Remaining seconds.
    pub remaining: i32,
    /// Next think time.
    pub next_think: f64,
    /// Paused.
    pub paused: bool,
    /// Teams locked.
    pub teams_locked: bool,
}

/// LMCTF match checkpoint (`LmctfMatch::capture`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LmctfMatchCheckpoint {
    /// Pending map.
    pub pending_map: Option<LmctfMapChange>,
    /// Phase.
    pub phase: LmctfMatchPhase,
    /// Remaining seconds.
    pub remaining: i32,
    /// Next think time.
    pub next_think: f64,
    /// Paused.
    pub paused: bool,
    /// Teams locked.
    pub teams_locked: bool,
}

/// LMCTF match (`LmctfMatch`).
#[derive(Debug, Clone, Copy)]
pub struct LmctfMatch {
    /// Session hooks.
    pub hooks: LmctfHooks,
}

impl LmctfMatch {
    /// Whether a match state can score (`canScore`).
    pub fn can_score_state(state: &LmctfMatchState) -> bool {
        state.phase != LmctfMatchPhase::Countdown && state.phase != LmctfMatchPhase::Over
    }

    /// Toggle pause (`togglePause`).
    pub fn toggle_pause(&self, game: &mut Q2GameServices) {
        game.lmctf.match_state.paused = !game.lmctf.match_state.paused;
        if game.lmctf.rules.auto_lock {
            game.lmctf.match_state.teams_locked = !game.lmctf.match_state.paused;
        }
        let text = if game.lmctf.match_state.paused {
            "Game Paused\n"
        } else {
            "Game Unpaused\n"
        }
        .to_string();
        let actors: Vec<ActorId> = game.lmctf.states.keys().cloned().collect();
        for actor in actors {
            if game.entity(&actor).is_some() {
                game.host_emit(Q2PresentationEvent::CenterPrint {
                    actor: actor.clone(),
                    text: text.clone(),
                    instant: false,
                    duration_seconds: None,
                });
            }
        }
        game.host.diagnostic(&text);
    }

    /// Change the map (`changeMap`).
    pub fn change_map(&self, game: &mut Q2GameServices, map: &str, countdown: bool) {
        self.stop(game);
        game.lmctf.match_state.pending_map = Some(LmctfMapChange {
            map: map.to_string(),
            countdown,
        });
        if countdown {
            game.lmctf.match_state.phase = LmctfMatchPhase::Countdown;
        }
        let actors: Vec<ActorId> = game.lmctf.states.keys().cloned().collect();
        for actor in actors {
            if let Some(state) = game.lmctf.states.get_mut(&actor) {
                state.statistics.clear();
            }
            if let Some(player) = (self.hooks.player)(actor, game) {
                player.score = 0;
            }
        }
    }

    /// Whether the match can score (`canScore`).
    pub fn can_score(&self, game: &Q2GameServices) -> bool {
        Self::can_score_state(&game.lmctf.match_state)
    }

    /// Start the match (`start`).
    pub fn start(&self, game: &mut Q2GameServices) {
        game.lmctf.match_state.phase = LmctfMatchPhase::Countdown;
        game.lmctf.match_state.remaining = game.lmctf.rules.countdown_seconds.trunc() as i32;
        game.lmctf.match_state.next_think = game.now() + 1.0;
        if game.lmctf.rules.auto_lock {
            game.lmctf.match_state.teams_locked = true;
        }
    }

    /// Stop the match (`stop`).
    pub fn stop(&self, game: &mut Q2GameServices) {
        game.lmctf.match_state.phase = LmctfMatchPhase::None;
        if game.lmctf.rules.auto_lock {
            game.lmctf.match_state.teams_locked = false;
        }
    }

    /// Run the match frame (`frame`).
    pub fn frame(&self, game: &mut Q2GameServices) {
        if game.lmctf.match_state.pending_map.is_some()
            || game.lmctf.match_state.phase == LmctfMatchPhase::None
            || game.now() < game.lmctf.match_state.next_think
        {
            return;
        }
        game.lmctf.match_state.next_think = game.now() + 1.0;
        if game.lmctf.match_state.paused {
            return;
        }
        if game.lmctf.match_state.phase == LmctfMatchPhase::Countdown {
            if [60, 30, 15, 10].contains(&game.lmctf.match_state.remaining) {
                let remaining = game.lmctf.match_state.remaining;
                lmctf_print(game, &format!("{remaining} seconds until match begins.\n"), None);
            }
            if game.lmctf.match_state.remaining <= 0 {
                let actors: Vec<ActorId> = game.lmctf.states.keys().cloned().collect();
                for actor in actors {
                    let spectator = (self.hooks.player)(actor.clone(), game)
                        .map(|player| player.spectator)
                        .unwrap_or(true);
                    if spectator || game.entity(&actor).is_none() {
                        continue;
                    }
                    let origin = game.body_of(actor.clone()).origin;
                    game.damage(
                        actor.clone(),
                        actor.clone(),
                        Some(actor.clone()),
                        100000.0,
                        0.0,
                        Vec3::default(),
                        origin,
                        Vec3::default(),
                        23,
                        32,
                        None,
                    );
                    if let Some(state) = game.lmctf.states.get_mut(&actor) {
                        state.statistics.clear();
                        state.spawn_state = 0;
                    }
                    if let Some(player) = (self.hooks.player)(actor, game) {
                        player.score = 0;
                    }
                }
                game.lmctf.match_state.phase = LmctfMatchPhase::Inplay;
                game.lmctf.match_state.remaining = game.lmctf.rules.time_limit_minutes.trunc() as i32 * 60;
                let remaining = game.lmctf.match_state.remaining;
                lmctf_print(game, &format!("{} minutes until match ends.\n", remaining / 60), None);
            }
        } else if game.lmctf.match_state.phase == LmctfMatchPhase::Inplay {
            let frag_limit = game.lmctf.rules.frag_limit;
            if game.lmctf.match_state.remaining > 10
                && frag_limit != 0
                && game
                    .lmctf
                    .states
                    .values()
                    .any(|state| state.statistics.get("score").copied().unwrap_or(0.0) >= f64::from(frag_limit))
            {
                game.lmctf.match_state.remaining = 10;
            }
            if game.lmctf.match_state.remaining <= 0 {
                let mut red = 0.0;
                let mut blue = 0.0;
                for state in game.lmctf.states.values() {
                    if state.team == 1 {
                        red += state.statistics.get("score").copied().unwrap_or(0.0);
                    } else if state.team == 2 {
                        blue += state.statistics.get("score").copied().unwrap_or(0.0);
                    }
                }
                lmctf_print(
                    game,
                    if red > blue {
                        format!("Red: {red} beats blue: {blue}!\n")
                    } else if blue > red {
                        format!("Blue: {blue} beats red: {red}!\n")
                    } else {
                        format!("Tie game at {red}!\n")
                    }
                    .as_str(),
                    None,
                );
                game.lmctf.match_state.phase = LmctfMatchPhase::Over;
                game.lmctf.match_state.remaining = 300;
                game.lmctf.match_state.teams_locked = false;
                return;
            }
        } else if game.lmctf.match_state.remaining <= 0 {
            game.lmctf.match_state.phase = LmctfMatchPhase::None;
        }
        game.lmctf.match_state.remaining -= 1;
    }

    /// Capture the match (`capture`).
    pub fn capture(&self, game: &Q2GameServices) -> LmctfMatchCheckpoint {
        let state = &game.lmctf.match_state;
        LmctfMatchCheckpoint {
            pending_map: state.pending_map.clone(),
            phase: state.phase,
            remaining: state.remaining,
            next_think: state.next_think,
            paused: state.paused,
            teams_locked: state.teams_locked,
        }
    }

    /// Restore the match (`restore`).
    pub fn restore(&self, game: &mut Q2GameServices, saved: &LmctfMatchCheckpoint) {
        game.lmctf.match_state.pending_map = saved.pending_map.clone();
        game.lmctf.match_state.phase = saved.phase;
        game.lmctf.match_state.remaining = saved.remaining;
        game.lmctf.match_state.next_think = saved.next_think;
        game.lmctf.match_state.paused = saved.paused;
        game.lmctf.match_state.teams_locked = saved.teams_locked;
    }
}
