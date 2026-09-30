//! Q2 LMCTF vote (`src/content/q2/multiplayer/lmctf/vote.ts`).
//!
//! LM_CTF 6.0 g_vote.c: implemented skip-level election. GPL-2.0-or-later.

use qa_core::identity::ActorId;

use crate::q2::foundation::host::Q2GameServices;

use super::types::{LmctfEvent, LmctfHooks, LmctfMenuEntry, lmctf_name, lmctf_player, lmctf_print};

/// LMCTF vote checkpoint (`LmctfVote::capture`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LmctfVoteCheckpoint {
    /// Start time.
    pub started_at: Option<f64>,
}

/// LMCTF vote (`LmctfVote`).
#[derive(Debug, Clone, Copy)]
pub struct LmctfVote {
    /// Session hooks.
    pub hooks: LmctfHooks,
}

impl LmctfVote {
    /// Connected player actors (`players`).
    fn players(&self, game: &mut Q2GameServices) -> Vec<ActorId> {
        let actors: Vec<ActorId> = game.lmctf.states.keys().cloned().collect();
        let mut out = Vec::new();
        for actor in actors {
            let connected = (self.hooks.player)(actor.clone(), game).map(|player| player.connected).unwrap_or(false);
            let is_player = game.entity(&actor).map(|entity| entity.classname == "player").unwrap_or(false);
            if connected && is_player {
                out.push(actor);
            }
        }
        out
    }

    /// Show the vote menu (`menu`).
    pub fn menu(&self, actor: ActorId, game: &mut Q2GameServices) {
        let ballot = lmctf_player(game, &actor).extra_flags;
        let entries = if game.lmctf.vote_started.is_none() {
            vec![
                LmctfMenuEntry { label: "Skip to next map".to_string(), command: Some("lmctf-vote skip".to_string()) },
                LmctfMenuEntry { label: "Vote:     Idle".to_string(), command: None },
            ]
        } else {
            vec![
                LmctfMenuEntry { label: "Vote YES".to_string(), command: Some("voteyes".to_string()) },
                LmctfMenuEntry { label: "Vote NO".to_string(), command: Some("voteno".to_string()) },
                LmctfMenuEntry { label: "Vote:     Started".to_string(), command: None },
                LmctfMenuEntry {
                    label: if ballot & 64 != 0 {
                        "You have voted YES"
                    } else if ballot & 128 != 0 {
                        "You have voted NO"
                    } else {
                        "You have not voted"
                    }
                    .to_string(),
                    command: None,
                },
            ]
        };
        (self.hooks.emit)(game, LmctfEvent::Menu { actor, title: "LMCTF Vote Menu".to_string(), entries });
    }

    /// Start a vote (`start`).
    pub fn start(&self, actor: ActorId, game: &mut Q2GameServices) {
        let players = self.players(game);
        for player in &players {
            if let Some(state) = game.lmctf.states.get_mut(player) {
                state.extra_flags &= !192;
            }
        }
        if players.len() < 4 {
            lmctf_print(game, "You need at least four players on the server to initiate a vote\n", Some(actor.clone()));
        } else if game.lmctf.vote_started.is_some() {
            lmctf_print(game, "Vote has already been started\n", Some(actor.clone()));
        } else {
            game.lmctf.vote_started = Some(game.now());
            lmctf_player(game, &actor).extra_flags |= 64;
            let name = lmctf_name(game, &actor);
            lmctf_print(game, &format!("{name} started vote to skip level.\n"), None);
            if game.entity(&actor).is_some() {
                game.sound(&actor, "misc/secret.wav", 3, 1.0, 0.0);
            }
        }
        self.menu(actor, game);
    }

    /// Cast a ballot (`ballot`).
    pub fn ballot(&self, actor: ActorId, game: &mut Q2GameServices, yes: bool) {
        if game.lmctf.vote_started.is_none() {
            lmctf_print(game, "A vote has not been initiated.\n", Some(actor));
            return;
        }
        let state = lmctf_player(game, &actor);
        state.extra_flags = state.extra_flags & !192 | if yes { 64 } else { 128 };
        lmctf_print(game, &format!("You have voted {}\n", if yes { "YES" } else { "NO" }), Some(actor.clone()));
        self.menu(actor, game);
    }

    /// Run the vote frame (`frame`).
    pub fn frame(&self, game: &mut Q2GameServices) {
        let Some(started) = game.lmctf.vote_started else {
            return;
        };
        if game.now() <= started + 30.0 {
            return;
        }
        game.lmctf.vote_started = None;
        lmctf_print(game, "Vote session has ended\n", None);
        let mut yes = 0;
        let mut no = 0;
        let mut abstained = 0;
        for player in self.players(game) {
            let flags = game.lmctf.states.get(&player).map(|state| state.extra_flags).unwrap_or(0);
            if flags & 64 != 0 {
                yes += 1;
            } else if flags & 128 != 0 {
                no += 1;
            } else {
                abstained += 1;
            }
        }
        lmctf_print(game, &format!("VOTE RESULT: YES:{yes}  NO:{no}  Abstained:{abstained}\n"), None);
        let total = yes + no;
        if total < 2 {
            lmctf_print(game, "Vote Fails: you need at least 2 ballots cast!\n", None);
            return;
        }
        let mut percentage = yes * 100 / total;
        if yes * 100 % total > total >> 1 {
            percentage += 1;
        }
        if percentage < 75 {
            lmctf_print(game, &format!("Vote Fails with {} percent majority\n", 100 - percentage), None);
            return;
        }
        lmctf_print(game, &format!("Vote to skip level Passes with {percentage} percent majority\n"), None);
        (self.hooks.end_level)(game, None);
    }

    /// Capture the vote (`capture`).
    pub fn capture(&self, game: &Q2GameServices) -> LmctfVoteCheckpoint {
        LmctfVoteCheckpoint { started_at: game.lmctf.vote_started }
    }

    /// Restore the vote (`restore`).
    pub fn restore(&self, game: &mut Q2GameServices, saved: &LmctfVoteCheckpoint) {
        game.lmctf.vote_started = saved.started_at;
    }
}
