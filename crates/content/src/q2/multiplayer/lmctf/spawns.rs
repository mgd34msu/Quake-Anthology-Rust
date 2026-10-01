//! Q2 LMCTF spawns (`src/content/q2/multiplayer/lmctf/spawns.ts`).
//!
//! LM_CTF p_client.c SelectTeamSpawnPoint/SelectAnySpawnPoint.
//! GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::{add3, vec3, Vec3};

use crate::q2::base::player::spawns::{q2_entities_named, q2_players_range, select_q2_spawn};
use crate::q2::foundation::host::Q2GameServices;

use super::types::lmctf_player;

/// Select the team spawn (`lmctfTeamSpawn`).
pub fn lmctf_team_spawn(game: &mut Q2GameServices, entity: &ActorId) -> Option<ActorId> {
    let team = lmctf_player(game, entity).team;
    let spots = q2_entities_named(
        game,
        if team == 1 {
            "info_player_red"
        } else {
            "info_player_blue"
        },
    );
    let mut best = None;
    let mut distance = 0.0;
    for spot in &spots {
        let current = q2_players_range(game, spot.clone());
        if current > distance {
            best = Some(spot.clone());
            distance = current;
        }
    }
    best.or_else(|| spots.into_iter().next())
}

/// Select an LMCTF spawn (`selectLmctfSpawn`).
pub fn select_lmctf_spawn(entity: ActorId, game: &mut Q2GameServices) -> (Vec3, Vec3) {
    let hooks = super::lmctf_hooks(game);
    let snapshot = (hooks.player)(entity.clone(), game).map(|player| player.clone());
    let Some(snapshot) = snapshot else {
        panic!("LMCTF spawn needs the admitted source player");
    };
    let mut spot = if lmctf_player(game, &entity).spawn_state == 0 {
        lmctf_team_spawn(game, &entity)
    } else {
        None
    };
    lmctf_player(game, &entity).spawn_state = 1;
    if spot.is_none() && !q2_entities_named(game, "info_player_deathmatch").is_empty() {
        let deathmatch = select_q2_spawn(game, &snapshot, "");
        let team = lmctf_team_spawn(game, &entity);
        spot = Some(match team {
            Some(team) if q2_players_range(game, deathmatch.clone()) <= q2_players_range(game, team.clone()) => team,
            _ => deathmatch,
        });
    }
    if spot.is_none() {
        spot = q2_entities_named(game, "info_flag_red")
            .into_iter()
            .next()
            .or_else(|| q2_entities_named(game, "info_flag_blue").into_iter().next());
    }
    let spot = spot.unwrap_or_else(|| select_q2_spawn(game, &snapshot, ""));
    let body = game.body_of(spot);
    (add3(body.origin, vec3(0.0, 0.0, 9.0)), body.angles)
}
