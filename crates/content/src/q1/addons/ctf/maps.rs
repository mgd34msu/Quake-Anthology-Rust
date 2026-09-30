//! Q1 CTF map rules, vote exits and match limits (src/content/q1/addons/ctf/maps.ts).

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::q1::addons::context::{add_frame_tick, addon_cvar, init_trigger, set_addon_number};
use crate::q1::addons::ctf::runes::start_runes;
use crate::q1::addons::ctf::state::{
    ctf_announce, ctf_body, ctf_number, ctf_set, ctf_start_map, ctf_world, with_ctf_services,
};
use crate::q1::addons::ctf::types::CtfTeam;
use crate::q1::base::provider::{level_begin, q1_base_classnames};
use crate::q1::foundation::callbacks::{Q1ActionHandler, Q1CallbackHandlers, Q1TouchHandler};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::{BodyPatch, TouchSurface};
use crate::q1::foundation::spawns::{spawn_map_actor, spawn_teledeath, spawn_teleport_fog};
use crate::q1::foundation::types::{vadd, vectors, vscale, Q1MessageArg, Q1MoveType, Q1Solid, ZERO};
use crate::q1::{q1_error, Q1Error};

/// Teleport a voter to the exit destination (`voteTeleport`).
fn vote_teleport(game: &mut Q1EntityServices, trigger: &ActorId, actor: &ActorId) -> Result<(), Q1Error> {
    let target_name = game
        .entity_ref(trigger)
        .map(|entity| entity.target.clone())
        .unwrap_or_default();
    let Some(target) = game.find(&target_name).into_iter().next() else {
        return Err(q1_error(format!("CTF vote exit has missing target {target_name}")));
    };
    let target_body = game.body(&target)?;
    let mangle = game.entity_ref(&target).map(|entity| entity.mangle).unwrap_or(ZERO);
    let origin = target_body.origin;
    let forward = vectors(mangle).forward;
    let body = ctf_body(game, actor)?;
    spawn_teleport_fog(game, body.origin)?;
    spawn_teleport_fog(game, vadd(origin, vscale(forward, 32.0)))?;
    spawn_teledeath(game, origin, actor)?;
    let id = actor.clone();
    let until = game.time + 0.7;
    with_ctf_services(game, |services| {
        services.teleport(&id, origin, mangle, vscale(forward, 300.0), until);
    })?;
    Ok(())
}

/// Handle a vote-exit touch (`voteTouch`).
pub fn vote_touch(game: &mut Q1EntityServices, trigger: &ActorId, actor: &ActorId) -> Result<(), Q1Error> {
    let trigger = trigger.clone();
    let id = actor.clone();
    if !game.is_player(&id) || game.health(&id) <= 0.0 {
        return Ok(());
    }
    if with_ctf_services(game, |services| services.observer(&id))? {
        return Ok(());
    }
    if ctf_number(game, &id, "voted")? != 0.0 {
        if ctf_number(game, &id, "voted")? < game.time {
            game.message(Some(&id), "$qc_ctf_already_voted", true, Vec::new());
        }
        ctf_set(game, &id, "voted", game.time + 1.0)?;
        return vote_teleport(game, &trigger, &id);
    }
    ctf_set(game, &id, "voted", game.time + 1.0)?;
    game.use_targets(&trigger, Some(&id))?;
    let message = game
        .entity_ref(&trigger)
        .map(|entity| entity.message.clone())
        .unwrap_or_default();
    ctf_announce(game, "$qc_ctf_has_voted", Some(&id), &message)?;
    game.update_entity(&trigger, |entity| {
        entity.count += 1.0;
    })?;
    let mut leader: Option<ActorId> = None;
    for candidate in game.entity_ids() {
        let better = game.entity_ref(&candidate).is_some_and(|entity| {
            entity.classname == "trigger_voteexit"
                && candidate != trigger
                && entity.count
                    > leader
                        .as_ref()
                        .and_then(|leader| game.entity_ref(leader).map(|entity| entity.count))
                        .unwrap_or(0.0)
        });
        if better {
            leader = Some(candidate);
        }
    }
    let trigger_count = game.entity_ref(&trigger).map(|entity| entity.count).unwrap_or(0.0);
    let leader_count = leader
        .as_ref()
        .and_then(|leader| game.entity_ref(leader).map(|entity| entity.count))
        .unwrap_or(0.0);
    if trigger_count > leader_count || trigger_count == leader_count && game.host.random() > 0.5 {
        leader = Some(trigger.clone());
    }
    let world = ctf_world(game)?;
    game.update_entity(&world, |entity| {
        entity.references.insert(String::from("ctf.voteLeader"), leader.clone());
    })?;
    let exit_time = game
        .entity_ref(&world)
        .map(|entity| entity.number("ctf.voteExitTime"))
        .unwrap_or(0.0);
    if leader.is_some() && exit_time == 0.0 {
        set_addon_number(game, &world, "ctf.voteExitTime", game.time + 60.0)?;
    }
    vote_teleport(game, &trigger, &id)
}

/// Stage the next level (`nextLevel`).
fn next_level(game: &mut Q1EntityServices, map: &str) -> Result<(), Q1Error> {
    let world = ctf_world(game)?;
    set_addon_number(game, &world, "ctf.pregameOver", 1.0)?;
    let timer = game.create("ctf_nextlevel", None, None)?;
    let map = map.to_string();
    game.update_entity(&timer, |entity| {
        entity.fields.insert(String::from("map"), map);
    })?;
    game.schedule(&timer, 0.1, "ctf:nextlevel")
}

/// Enforce vote exits on `start` and match limits elsewhere
/// (`mapFrame`).
pub fn map_frame(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    if game.intermission.is_some() {
        return Ok(());
    }
    let world = ctf_world(game)?;
    let pregame_over = game
        .entity_ref(&world)
        .map(|entity| entity.number("ctf.pregameOver"))
        .unwrap_or(0.0);
    if pregame_over != 0.0 {
        return Ok(());
    }
    if ctf_start_map(game) {
        let leader = game
            .entity_ref(&world)
            .and_then(|entity| entity.references.get("ctf.voteLeader").cloned().flatten());
        let time = game
            .entity_ref(&world)
            .map(|entity| entity.number("ctf.voteExitTime"))
            .unwrap_or(0.0);
        if let Some(leader) = leader {
            if time != 0.0 && game.time > time {
                let map = game
                    .entity_ref(&leader)
                    .map(|entity| entity.text("map"))
                    .unwrap_or_default();
                next_level(game, &map)?;
            }
        }
        return Ok(());
    }
    let time_limit = addon_cvar(game, "timelimit")? * 60.0;
    let capture_limit = addon_cvar(game, "fraglimit")?;
    let red = with_ctf_services(game, |services| services.captures(CtfTeam::Red))?;
    let blue = with_ctf_services(game, |services| services.captures(CtfTeam::Blue))?;
    if !(time_limit != 0.0 && game.time >= time_limit)
        && !(capture_limit != 0.0 && (f64::from(red) >= capture_limit || f64::from(blue) >= capture_limit))
    {
        return Ok(());
    }
    for actor in (game.host.players)() {
        if red == blue {
            game.message(
                Some(&actor),
                "$qc_ks_match_tied",
                false,
                vec![Q1MessageArg::Number(f64::from(red))],
            );
        } else {
            game.message(
                Some(&actor),
                if red > blue {
                    "$qc_ks_red_won"
                } else {
                    "$qc_ks_red_lost"
                },
                false,
                vec![Q1MessageArg::Number(f64::from(red))],
            );
            game.message(
                Some(&actor),
                if blue > red {
                    "$qc_ks_blue_won"
                } else {
                    "$qc_ks_blue_lost"
                },
                false,
                vec![Q1MessageArg::Number(f64::from(blue))],
            );
        }
    }
    let maps = ["ctf1", "ctf2", "ctf3", "ctf4", "ctf5", "ctf6", "ctf7", "ctf8", "ctf9"];
    let next = maps
        .iter()
        .position(|map| *map == game.map_name)
        .map(|index| maps[(index + 1) % maps.len()])
        .unwrap_or("ctf1");
    next_level(game, next)
}

fn nextlevel_action(game: &mut Q1EntityServices, entity: &ActorId) -> Result<(), Q1Error> {
    let entity = entity.clone();
    let map = game
        .entity_ref(&entity)
        .map(|entity| entity.text("map"))
        .unwrap_or_default();
    level_begin(game, &map, None)?;
    game.remove(&entity)
}

fn frame_action(game: &mut Q1EntityServices, _entity: &ActorId) -> Result<(), Q1Error> {
    map_frame(game)
}

fn vote_touch_action(
    game: &mut Q1EntityServices,
    entity: &ActorId,
    actor: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    vote_touch(game, entity, actor)
}

fn changelevel_touch(
    game: &mut Q1EntityServices,
    entity: &ActorId,
    actor: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let entity = entity.clone();
    let id = actor.clone();
    let noexit = addon_cvar(game, "noexit")?;
    if !game.is_player(&id) || noexit == 1.0 || noexit == 2.0 && !ctf_start_map(game) {
        return Ok(());
    }
    ctf_announce(game, "$qc_exited", Some(&id), "")?;
    game.use_targets(&entity, Some(&id))?;
    let spawnflags = game.entity_ref(&entity).map(|entity| entity.spawnflags).unwrap_or(0);
    if spawnflags & 1 != 0 && game.options().deathmatch == 0 {
        let map = game
            .entity_ref(&entity)
            .map(|entity| entity.text("map"))
            .unwrap_or_default();
        game.travel(&map, Some(&id));
        return Ok(());
    }
    game.update_entity(&entity, |entity| {
        entity.touch = None;
    })?;
    game.schedule(&entity, 0.1, "ctf:nextlevel")
}

fn spawn_world(game: &mut Q1EntityServices, world: &ActorId) -> Result<(), Q1Error> {
    spawn_map_actor(game, world)?;
    add_frame_tick(game, world, "ctf:frame")?;
    Ok(())
}

fn spawn_vote_destination(game: &mut Q1EntityServices, entity: &ActorId) -> Result<(), Q1Error> {
    let targetname = game
        .entity_ref(entity)
        .map(|entity| entity.targetname.clone())
        .unwrap_or_default();
    if targetname.is_empty() {
        return Err(q1_error("CTF vote destination has no targetname"));
    }
    let body = game.body(entity)?;
    game.update_entity(entity, |entity| {
        entity.mangle = body.angles;
        entity.model.clear();
    })?;
    game.set_body(
        entity,
        &BodyPatch {
            angles: Some(ZERO),
            origin: Some(vadd(
                body.origin,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 27.0,
                },
            )),
            ..Default::default()
        },
    )
}

fn spawn_vote_exit(game: &mut Q1EntityServices, entity: &ActorId) -> Result<(), Q1Error> {
    let target = game
        .entity_ref(entity)
        .map(|entity| entity.target.clone())
        .unwrap_or_default();
    // The released QC has an unconditional objerror here; retain its
    // documented map behavior with the missing condition restored.
    if target.is_empty() {
        return Err(q1_error("CTF vote exit has no target"));
    }
    init_trigger(game, entity)?;
    let touch = game.named.touch("ctf:vote_touch")?;
    game.update_entity(entity, |entity| {
        entity.count = 0.0;
        entity.touch = Some(touch);
    })?;
    game.link(entity)
}

fn spawn_changelevel(game: &mut Q1EntityServices, entity: &ActorId) -> Result<(), Q1Error> {
    let map = game
        .entity_ref(entity)
        .map(|entity| entity.text("map"))
        .unwrap_or_default();
    if map.is_empty() {
        return Err(q1_error("CTF changelevel trigger has no map"));
    }
    init_trigger(game, entity)?;
    let touch = game.named.touch("ctf:changelevel_touch")?;
    game.update_entity(entity, |entity| {
        entity.touch = Some(touch);
    })?;
    game.link(entity)
}

fn spawn_ctf_wall(game: &mut Q1EntityServices, entity: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(entity, |entity| {
        entity.solid = Q1Solid::Bsp;
        entity.movement = Q1MoveType::Push;
    })?;
    game.set_body(
        entity,
        &BodyPatch {
            angles: Some(ZERO),
            ..Default::default()
        },
    )?;
    game.link(entity)
}

fn spawn_team_start(_game: &mut Q1EntityServices, _entity: &ActorId) -> Result<(), Q1Error> {
    Ok(())
}

fn spawn_deathmatch(game: &mut Q1EntityServices, _entity: &ActorId) -> Result<(), Q1Error> {
    if game.options().deathmatch != 0 {
        start_runes(game)?;
    }
    Ok(())
}

fn remove_entity(game: &mut Q1EntityServices, entity: &ActorId) -> Result<(), Q1Error> {
    game.remove(entity)
}

/// Monster classnames removed from CTF maps.
const CTF_REMOVED_MONSTERS: [&str; 16] = [
    "monster_army",
    "monster_dog",
    "monster_ogre",
    "monster_ogre_marksman",
    "monster_knight",
    "monster_hell_knight",
    "monster_wizard",
    "monster_demon1",
    "monster_shambler",
    "monster_zombie",
    "monster_tarbaby",
    "monster_fish",
    "monster_enforcer",
    "monster_shalrath",
    "monster_boss",
    "monster_oldone",
];

/// Register map callbacks and spawn handlers (`registerMaps`).
pub fn register_maps(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        "ctf:nextlevel",
        Q1CallbackHandlers {
            action: Some(nextlevel_action as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        "ctf:frame",
        Q1CallbackHandlers {
            action: Some(frame_action as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        "ctf:vote_touch",
        Q1CallbackHandlers {
            touch: Some(vote_touch_action as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        "ctf:changelevel_touch",
        Q1CallbackHandlers {
            touch: Some(changelevel_touch as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    game.register_spawn("worldspawn", spawn_world)?;
    game.register_spawn("info_vote_destination", spawn_vote_destination)?;
    game.register_spawn("trigger_voteexit", spawn_vote_exit)?;
    game.replace_spawn("trigger_changelevel", spawn_changelevel)?;
    game.register_spawn("func_ctf_wall", spawn_ctf_wall)?;
    game.register_spawn("info_player_team1", spawn_team_start)?;
    game.register_spawn("info_player_team2", spawn_team_start)?;
    game.register_spawn("info_player_deathmatch", spawn_deathmatch)?;
    let base = q1_base_classnames();
    for classname in CTF_REMOVED_MONSTERS {
        if base.contains(&classname) {
            game.replace_spawn(classname, remove_entity)?;
        } else {
            game.register_spawn(classname, remove_entity)?;
        }
    }
    Ok(())
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
        let world = game.create("worldspawn", None, None).expect("world");
        game.world = Some(world);
        let player = attach_test_player(game);
        (guard, player)
    }

    #[test]
    fn repeat_vote_teleports_without_recount() {
        let mut game = test_game();
        let (_guard, player) = setup(&mut game);
        register_maps(&mut game).expect("maps");
        let destination = game.create("info_vote_destination", None, None).expect("dest");
        game.update_entity(&destination, |entity| {
            entity.targetname = String::from("vote_a");
        })
        .expect("targetname");
        let trigger = game.create("trigger_voteexit", None, None).expect("trigger");
        game.update_entity(&trigger, |entity| {
            entity.target = String::from("vote_a");
            entity.message = String::from("ctf1");
            entity.count = 3.0;
        })
        .expect("vote exit");
        let voted_until = game.time + 100.0;
        ctf_set(&mut game, &player, "voted", voted_until).expect("voted");
        vote_touch(&mut game, &trigger, &player).expect("vote");
        assert_eq!(game.entity_ref(&trigger).map(|entity| entity.count), Some(3.0));
        assert_eq!(ctf_number(&game, &player, "voted"), Ok(game.time + 1.0));
    }

    #[test]
    fn map_frame_waits_for_limits() {
        let mut game = test_game();
        let (_guard, _player) = setup(&mut game);
        register_maps(&mut game).expect("maps");
        map_frame(&mut game).expect("frame");
        let world = ctf_world(&game).expect("world");
        assert_eq!(
            game.entity_ref(&world).map(|entity| entity.number("ctf.pregameOver")),
            Some(0.0)
        );
    }
}
