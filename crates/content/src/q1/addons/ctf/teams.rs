//! Q1 CTF team admission (src/content/q1/addons/ctf/teams.ts).

use qa_core::identity::{same_actor, ActorId};

use crate::q1::addons::context::{set_addon_player_number, set_combat_team};
use crate::q1::addons::ctf::state::{
    ctf_announce, ctf_last_team, ctf_number, ctf_owner, ctf_set, ctf_start_map, ctf_team, ctf_teamplay,
    ctf_teamplay_bits, ctf_world, team_number, with_ctf_services,
};
use crate::q1::addons::ctf::types::{CtfFlags, CtfPromptChoice, CtfTeam};
use crate::q1::base::provider::spawn_select;
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::gameplay::CombatTraits;
use crate::q1::Q1Error;

/// Admit an actor to a team, recoloring shirt and pants (`setTeam`).
pub fn set_team(game: &mut Q1EntityServices, actor: &ActorId, team: Option<CtfTeam>) -> Result<(), Q1Error> {
    let id = actor.clone();
    let owner = ctf_owner(game, &id)?;
    set_combat_team(game, &owner, team.map(CtfTeam::as_str))?;
    ctf_set(game, &id, "lastteam", team_number(team) as f64)?;
    let color = team.map_or(0, |team| team_number(Some(team)) - 1);
    with_ctf_services(game, |services| {
        services.colors(&id, color as i32, color as i32);
    })?;
    // Keep the synced stage policy fresh for team-gated callbacks.
    crate::q1::addons::ctf::state::sync_ctf_policy(game)
}

/// Assign the emptier team, breaking ties randomly (`checkTeam`).
pub fn check_team(game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
    let id = actor.clone();
    let current = ctf_team(game, &id);
    if ctf_number(game, &id, "lastteam")? >= 0.0 && current.is_some() {
        ctf_set(game, &id, "lastteam", team_number(current) as f64)?;
        return Ok(());
    }
    let mut red = 0;
    let mut blue = 0;
    for player in (game.host.players)() {
        if same_actor(&id, &player) {
            continue;
        }
        match ctf_team(game, &player) {
            Some(CtfTeam::Red) => red += 1,
            Some(CtfTeam::Blue) => blue += 1,
            None => {}
        }
    }
    let flip = game.host.random() < 0.5;
    set_team(
        game,
        &id,
        Some(if blue < red || blue == red && flip {
            CtfTeam::Blue
        } else {
            CtfTeam::Red
        }),
    )
}

/// Enforce team locks and re-admit team changers (`checkTeamLock`).
pub fn check_team_lock(game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
    let id = actor.clone();
    if ctf_teamplay(game)? < 0.0 {
        return Ok(());
    }
    if with_ctf_services(game, |services| services.observer(&id))? || ctf_start_map(game) {
        if ctf_number(game, &id, "lastteam")? != 1.0 {
            with_ctf_services(game, |services| services.colors(&id, 0, 0))?;
        }
        ctf_set(game, &id, "lastteam", 1.0)?;
        return Ok(());
    }
    if ctf_number(game, &id, "stuffColor")? != 0.0 {
        ctf_set(game, &id, "stuffColor", 0.0)?;
        let color = ctf_number(game, &id, "lastteam")? as i32 - 1;
        with_ctf_services(game, |services| services.colors(&id, color, color))?;
        return Ok(());
    }
    let current = ctf_team(game, &id);
    let previous = ctf_last_team(game, &id)?;
    if current.is_none() && ctf_number(game, &id, "lastteam")? == 0.0 {
        ctf_set(game, &id, "lastteam", -1.0)?;
    }
    if team_number(current) == ctf_number(game, &id, "lastteam")? as i64 {
        return Ok(());
    }
    if ctf_teamplay_bits(game)? & CtfFlags::STATIC_TEAMS != 0 && ctf_number(game, &id, "lastteam")? >= 0.0 {
        if let Some(previous) = previous {
            if ctf_number(game, &id, "suicideCount")? > 3.0 {
                with_ctf_services(game, |services| services.disconnect(&id))?;
            }
            let killed = ctf_number(game, &id, "killed")?;
            ctf_set(game, &id, "killed", if killed == 1.0 { 1.0 } else { 2.0 })?;
            clear_invulnerable(game, &id)?;
            game.damage(
                &id,
                Some(&id),
                Some(&id),
                1000.0,
                &Q1DamageParams {
                    death_type: String::from("ctf:teamchange"),
                    ..Default::default()
                },
            );
            ctf_set(game, &id, "suicideCount", ctf_number(game, &id, "suicideCount")? + 1.0)?;
            return set_team(game, &id, Some(previous));
        }
        ctf_set(game, &id, "lastteam", -50.0)?;
    }
    if ctf_number(game, &id, "lastteam")? > 0.0 {
        let killed = ctf_number(game, &id, "killed")?;
        ctf_set(game, &id, "killed", if killed == 1.0 { 1.0 } else { 2.0 })?;
        game.damage(
            &id,
            Some(&id),
            Some(&id),
            1000.0,
            &Q1DamageParams {
                death_type: String::from("ctf:teamchange"),
                ..Default::default()
            },
        );
    }
    let score = with_ctf_services(game, |services| services.score(&id))?;
    with_ctf_services(game, |services| services.add_score(&id, -score))?;
    check_team(game, &id)
}

/// Clear invulnerability while preserving the remaining traits.
fn clear_invulnerable(game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
    let owner = ctf_owner(game, actor)?;
    let Some(combat) = game.host.combat.read(actor) else {
        return Ok(());
    };
    game.host.combat.set_traits(
        &owner,
        CombatTraits {
            can_take_damage: combat.can_take_damage,
            mass: combat.mass,
            invulnerable: false,
            team: combat.team,
            no_knockback: combat.no_knockback,
        },
    )
}

/// Select a spawn point for an actor (`spawnPoint`).
pub fn spawn_point(game: &mut Q1EntityServices, actor: &ActorId) -> Result<Option<ActorId>, Q1Error> {
    let id = actor.clone();
    let team = ctf_team(game, &id);
    let entities = game.entity_ids();
    if let Some(test) = entities.iter().find(|entry| {
        game.entity_ref(entry)
            .is_some_and(|entity| entity.classname == "testplayerstart")
    }) {
        return Ok(Some(test.clone()));
    }
    if game.options().coop || game.options().deathmatch == 0 {
        return spawn_select(game, true);
    }
    let killed = ctf_number(game, &id, "killed")?;
    let classname = if ctf_start_map(game) && killed != 0.0 {
        "info_vote_destination"
    } else if killed == 0.0 && team.is_some() {
        if team == Some(CtfTeam::Red) {
            "info_player_team1"
        } else {
            "info_player_team2"
        }
    } else {
        "info_player_deathmatch"
    };
    let spots: Vec<ActorId> = entities
        .into_iter()
        .filter(|entry| {
            game.entity_ref(entry)
                .is_some_and(|entity| entity.classname == classname)
        })
        .collect();
    let world = ctf_world(game)?;
    let key = format!("ctf.spawn.{classname}");
    let last = game
        .entity_ref(&world)
        .and_then(|entity| entity.references.get(&key).cloned().flatten());
    let index = last
        .as_ref()
        .and_then(|last| spots.iter().position(|spot| same_actor(spot, last)))
        .map_or(-1, |index| index as i32);
    let Some(spot) = spots.get(((index + 1) as usize) % spots.len().max(1)).cloned() else {
        return Ok(game
            .entity_ids()
            .into_iter()
            .find(|entry| {
                game.entity_ref(entry)
                    .is_some_and(|entity| entity.classname == "info_player_deathmatch")
            })
            .or_else(|| {
                game.entity_ids().into_iter().find(|entry| {
                    game.entity_ref(entry)
                        .is_some_and(|entity| entity.classname == "info_player_start")
                })
            }));
    };
    game.update_entity(&world, |entity| {
        entity.references.insert(key, Some(spot.clone()));
    })?;
    Ok(Some(spot))
}

/// Show the team admission prompt (`showTeamPrompt`).
pub fn show_team_prompt(game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
    let id = actor.clone();
    if with_ctf_services(game, |services| services.prompt_supported(&id))? {
        with_ctf_services(game, |services| {
            services.prompt(
                &id,
                "$qc_ctf_intro",
                &[
                    CtfPromptChoice {
                        label: String::from("$qc_ctf_intro_auto"),
                        impulse: 103,
                    },
                    CtfPromptChoice {
                        label: String::from("$qc_ctf_intro_red"),
                        impulse: 101,
                    },
                    CtfPromptChoice {
                        label: String::from("$qc_ctf_intro_blue"),
                        impulse: 102,
                    },
                    CtfPromptChoice {
                        label: String::from("$qc_ctf_intro_observer"),
                        impulse: 104,
                    },
                ],
            );
        })?;
        return Ok(());
    }
    game.message(
        Some(&id),
        "Welcome!\nRunning ThreeWave CTF 5.0\n\nCapture the Flag!\n\nPress 1 for RED team\nPress 2 for BLUE team\nOr press JUMP for automatic team\n",
        true,
        Vec::new(),
    );
    Ok(())
}

/// Handle team admission impulses, returning whether one was consumed
/// (`observerImpulse`).
pub fn observer_impulse(
    game: &mut Q1EntityServices,
    actor: &ActorId,
    forced_impulse: Option<i32>,
) -> Result<bool, Q1Error> {
    let id = actor.clone();
    let input = with_ctf_services(game, |services| services.input(&id))?;
    let impulse = forced_impulse.unwrap_or(input.impulse);
    let observer = with_ctf_services(game, |services| services.observer(&id))?;
    let prompt_supported = with_ctf_services(game, |services| services.prompt_supported(&id))?;
    if !(100..=104).contains(&impulse) && !(!prompt_supported && observer && ((1..=3).contains(&impulse) || input.jump))
    {
        return Ok(false);
    }
    observer_impulse_inner(game, &id, impulse, observer)
}

/// Inner admission switch shared after the impulse gate.
fn observer_impulse_inner(
    game: &mut Q1EntityServices,
    actor: &ActorId,
    impulse: i32,
    observer: bool,
) -> Result<bool, Q1Error> {
    if impulse == 100 && ctf_teamplay_bits(game)? & CtfFlags::STATIC_TEAMS != 0 {
        game.message(Some(actor), "$qc_ctf_teams_locked", true, Vec::new());
        with_ctf_services(game, |services| services.consume_impulse(actor))?;
        return Ok(true);
    }
    if !observer {
        game.damage(
            actor,
            Some(actor),
            Some(actor),
            1000.0,
            &Q1DamageParams {
                death_type: String::from("ctf:teamchange"),
                ..Default::default()
            },
        );
    }
    with_ctf_services(game, |services| services.set_observer(actor, false))?;
    ctf_set(game, actor, "killed", 0.0)?;
    if impulse == 100 || impulse == 104 {
        set_team(game, actor, None)?;
        with_ctf_services(game, |services| services.set_observer(actor, true))?;
    } else if impulse == 1 || impulse == 101 {
        set_team(game, actor, Some(CtfTeam::Red))?;
    } else if impulse == 2 || impulse == 102 {
        set_team(game, actor, Some(CtfTeam::Blue))?;
    } else if impulse == 103 {
        set_addon_player_number(game, actor, "ctf.lastteam", -50.0)?;
        check_team(game, actor)?;
    }
    with_ctf_services(game, |services| services.clear_prompt(actor))?;
    if ctf_last_team(game, actor)? == Some(CtfTeam::Red) {
        ctf_announce(game, "$qc_ks_joined_red", Some(actor), "")?;
    } else if ctf_last_team(game, actor)? == Some(CtfTeam::Blue) {
        ctf_announce(game, "$qc_ks_joined_blue", Some(actor), "")?;
    }
    with_ctf_services(game, |services| services.consume_impulse(actor))?;
    ctf_set(game, actor, "stuffColor", 1.0)?;
    let spot = spawn_point(game, actor)?;
    with_ctf_services(game, |services| services.respawn(actor, spot.as_ref()))?;
    if impulse == 100 {
        show_team_prompt(game, actor)?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::{attach_test_player, register_test_addons, Q1AddonProgram};
    use crate::q1::addons::ctf::state::register_ctf_state;
    use crate::q1::addons::ctf::types::FakeCtfServices;
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::missionpacks::types::test_game;

    fn setup(game: &mut Q1EntityServices) -> (Q1BaseGuard, ActorId) {
        let guard = Q1BaseGuard::register(game, Q1BaseOptions::default()).expect("base");
        register_test_addons(game, Q1AddonProgram::Ctf);
        let (services, _) = FakeCtfServices::new();
        register_ctf_state(game, Box::new(services), true, false);
        let player = attach_test_player(game);
        (guard, player)
    }

    #[test]
    fn check_team_assigns_a_team() {
        let mut game = test_game();
        let (_guard, player) = setup(&mut game);
        check_team(&mut game, &player).expect("check");
        assert!(ctf_team(&game, &player).is_some());
        assert!(ctf_last_team(&game, &player).expect("last").is_some());
    }

    #[test]
    fn observer_impulse_joins_red() {
        let mut game = test_game();
        let (_guard, player) = setup(&mut game);
        with_ctf_services(&game, |services| services.set_observer(&player, true)).expect("services");
        let start = game.create("info_player_start", None, None).expect("start");
        game.spawn_entity(&start, None).expect("spawn start");
        assert_eq!(observer_impulse(&mut game, &player, Some(101)), Ok(true));
        assert_eq!(ctf_team(&game, &player), Some(CtfTeam::Red));
    }
}
