//! Q2 LMCTF presentation (`src/content/q2/multiplayer/lmctf/presentation.ts`).
//!
//! LM_CTF 6.0 p_hud.c compact team scoreboard protocol. GPL-2.0-or-later.

use std::cmp::Ordering;

use qa_core::identity::ActorId;

use crate::q2::foundation::host::Q2GameServices;

use super::types::{LmctfEvent, LmctfMenuEntry, LmctfScoreRow};

/// Show the LMCTF scoreboard (`lmctfScoreboard`).
pub fn lmctf_scoreboard(game: &mut Q2GameServices, actor: ActorId) {
    let hooks = super::lmctf_hooks(game);
    let mut rows = Vec::new();
    let actors: Vec<ActorId> = game.lmctf.states.keys().cloned().collect();
    for id in actors {
        let score = game.lmctf.states.get(&id).and_then(|state| state.statistics.get("score").copied()).unwrap_or(0.0);
        let team = game.lmctf.states.get(&id).map(|state| state.team).unwrap_or(0);
        let Some(player) = (hooks.player)(id.clone(), game) else {
            continue;
        };
        if !player.connected {
            continue;
        }
        rows.push(LmctfScoreRow { actor: id, slot: player.slot, name: player.name.clone(), team, score, ping: 999.min(player.ping) });
    }
    rows.sort_by(|left: &LmctfScoreRow, right: &LmctfScoreRow| {
        right.score.partial_cmp(&left.score).unwrap_or(Ordering::Equal)
    });
    let mut layout = "xv 0 yv 32 string2 \"Scr Png Name        \" xv 0 yv 40 string2 \"------------------- \" xv 160 yv 32 string2 \"Scr Png Name        \" xv 160 yv 40 string2 \"------------------- \" ".to_string();
    for team in [1u8, 2u8] {
        for (index, row) in rows.iter().filter(|row| row.team == team).take(21).enumerate() {
            let entry = format!("ctf {} {} {} {} {} ", if team == 1 { 0 } else { 160 }, 48 + index * 8, row.slot, row.score, row.ping);
            if layout.len() + entry.len() <= 1024 {
                layout.push_str(&entry);
            }
        }
    }
    (hooks.emit)(game, LmctfEvent::Scoreboard { actor, rows, layout });
}

/// Show the LMCTF menu (`lmctfMenu`).
pub fn lmctf_menu(game: &mut Q2GameServices, actor: ActorId) {
    let hooks = super::lmctf_hooks(game);
    let mut entries = vec![
        LmctfMenuEntry { label: "Join Red Team".to_string(), command: Some("team red".to_string()) },
        LmctfMenuEntry { label: "Join Blue Team".to_string(), command: Some("team blue".to_string()) },
        LmctfMenuEntry { label: "Become Observer".to_string(), command: Some("observe".to_string()) },
    ];
    if game.lmctf.rules.ctf_flags & 32768 == 0 {
        entries.push(LmctfMenuEntry { label: "Voting Menu".to_string(), command: Some("lmctf-vote".to_string()) });
    }
    (hooks.emit)(game, LmctfEvent::Menu { actor, title: "LMCTF Menu".to_string(), entries });
}
