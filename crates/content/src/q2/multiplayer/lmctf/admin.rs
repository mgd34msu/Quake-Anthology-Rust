//! Q2 LMCTF admin (`src/content/q2/multiplayer/lmctf/admin.ts`).
//!
//! LM_CTF 6.0 g_cmds.c referee commands. GPL-2.0-or-later.

use qa_core::identity::ActorId;

use crate::q2::foundation::host::Q2GameServices;

use super::match_::{LmctfMatch, LmctfMatchPhase};
use super::types::{lmctf_name, lmctf_player, lmctf_print};

/// Run an LMCTF admin command (`lmctfAdminCommand`).
pub fn lmctf_admin_command(game: &mut Q2GameServices, entity: ActorId, name: &str, args: &[String]) -> bool {
    let extra = lmctf_player(game, &entity).extra_flags;
    if name == "pausematch" || name == "unpausematch" || name == "pause_match" {
        if name != "pause_match" || extra & 2 != 0 {
            LmctfMatch {
                hooks: super::lmctf_hooks(game),
            }
            .toggle_pause(game);
        }
        return true;
    }
    if name == "referee" {
        let password = args.join(" ");
        if password == game.lmctf.rules.rcon_password {
            if password.is_empty() {
                lmctf_print(game, "Rcon Mode is off\n", Some(entity));
            } else {
                lmctf_player(game, &entity).extra_flags |= 6;
                lmctf_print(game, "You are now an Rcon\n", Some(entity));
            }
        } else if password == game.lmctf.rules.ref_password {
            if password.is_empty() {
                lmctf_print(game, "Referee Mode is off\n", Some(entity));
            } else {
                lmctf_player(game, &entity).extra_flags = (extra | 2) & !4;
                lmctf_print(game, "You are now a Referee\n", Some(entity));
            }
        } else {
            lmctf_player(game, &entity).extra_flags = extra & !6;
            lmctf_print(game, "Incorrect Referee Password\n", Some(entity));
        }
        return true;
    }
    if name == "match" {
        if extra & 2 == 0 {
            lmctf_print(game, "You are not a Referee\n", Some(entity));
            return true;
        }
        let map = args.join(" ");
        if map.is_empty() {
            lmctf_print(game, "USAGE: match <mapname>\n", Some(entity));
        } else if !game.lmctf.rules.map_list.iter().any(|entry| entry == &map) {
            lmctf_print(game, &format!("{map} is not a map from the maplist.\n"), Some(entity));
        } else {
            lmctf_print(game, "Match countdown beginning.\n", Some(entity.clone()));
            LmctfMatch {
                hooks: super::lmctf_hooks(game),
            }
            .change_map(game, &map, true);
        }
        return true;
    }
    if name != "lock" && name != "unlock" && name != "startmatch" && name != "stopmatch" && name != "gotomap" {
        return false;
    }
    if extra & 2 == 0 {
        lmctf_print(game, "Referee-only command denied.\n", Some(entity));
        return true;
    }
    let matched = LmctfMatch {
        hooks: super::lmctf_hooks(game),
    };
    match name {
        "lock" | "unlock" => {
            game.lmctf.match_state.teams_locked = !game.lmctf.match_state.teams_locked;
            let locked = game.lmctf.match_state.teams_locked;
            lmctf_print(
                game,
                &format!("Teams are now {}locked\n", if locked { "" } else { "un" }),
                None,
            );
        }
        "startmatch" => {
            if game.lmctf.match_state.phase != LmctfMatchPhase::None {
                lmctf_print(game, "Match already running, stop it first\n", Some(entity));
            } else {
                matched.start(game);
            }
        }
        "stopmatch" => {
            if game.lmctf.match_state.phase == LmctfMatchPhase::None {
                lmctf_print(game, "No match running\n", Some(entity));
            } else {
                matched.stop(game);
                let name = lmctf_name(game, &entity);
                lmctf_print(game, &format!("Match stopped by {name}\n"), None);
            }
        }
        "gotomap" => {
            let map = args.join(" ").to_lowercase();
            if !map.is_empty() {
                if game.lmctf.rules.map_list.iter().any(|entry| entry == &map) {
                    matched.change_map(game, &map, false);
                } else {
                    lmctf_print(game, &format!("{map} is not a map from the maplist.\n"), Some(entity));
                }
            }
        }
        _ => {}
    }
    true
}
