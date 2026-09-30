//! Q1 CTF flag state, carrier offsets and capture assists (src/content/q1/addons/ctf/flags.ts).

use qa_core::identity::{same_actor, ActorId};
use qa_core::math::Vec3;

use crate::q1::addons::context::set_addon_number;
use crate::q1::addons::ctf::state::{
    ctf_announce, ctf_body, ctf_carried, ctf_flag, ctf_flag_team, ctf_grant, ctf_last_team, ctf_number, ctf_owner,
    ctf_set, ctf_team, ctf_update, ctf_world, opposite, team_number, with_ctf_services,
};
use crate::q1::addons::ctf::types::{CtfTeam, CTF_FLAG_BOUNDS};
use crate::q1::foundation::callbacks::{Q1ActionHandler, Q1CallbackHandlers, Q1TouchHandler};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::{BodyPatch, TouchSurface};
use crate::q1::foundation::types::{
    vadd, vectors, vscale, vsub, Q1MoveType, Q1Solid, Q1SoundChannel, Q1TraceRequest, ZERO,
};
use crate::q1::{q1_error, Q1Error};

/// Return a flag to its base (`returnFlag`).
pub fn return_flag(game: &mut Q1EntityServices, flag: &ActorId, announce: bool) -> Result<(), Q1Error> {
    let flag = flag.clone();
    let team = ctf_flag_team(game, &flag);
    let (base, mangle) = {
        let entity = game.entity_ref(&flag).ok_or_else(|| q1_error("Missing Q1 entity"))?;
        (entity.vector("ctf.base"), entity.mangle)
    };
    game.update_entity(&flag, |entity| {
        entity.movement = Q1MoveType::Toss;
        entity.solid = Q1Solid::Trigger;
        entity.count = 0.0;
        entity.owner = None;
    })?;
    game.set_body(
        &flag,
        &BodyPatch {
            origin: Some(base),
            angles: Some(mangle),
            ..Default::default()
        },
    )?;
    game.link(&flag)?;
    game.sound(&flag, "items/itembk2.wav", Q1SoundChannel::Voice, 1.0, 1.0)?;
    if announce {
        for player in (game.host.players)() {
            game.message(
                Some(&player),
                if ctf_team(game, &player) == Some(team) {
                    "$qc_ctf_your_returned"
                } else {
                    "$qc_ctf_enemy_returned"
                },
                true,
                Vec::new(),
            );
        }
    }
    Ok(())
}

/// Drop the flag carried by an actor (`dropFlag`).
pub fn drop_flag(game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
    let id = actor.clone();
    let Some(flag) = ctf_carried(game, &id) else {
        return Ok(());
    };
    let last = ctf_last_team(game, &id)?;
    ctf_announce(
        game,
        if last == Some(CtfTeam::Red) {
            "$qc_ks_blue_dropped"
        } else {
            "$qc_ks_red_dropped"
        },
        Some(&id),
        "",
    )?;
    with_ctf_services(game, |services| services.log(&id, "FLAG-DROP"))?;
    game.update_entity(&flag, |entity| {
        entity.count = 2.0;
        entity.movement = Q1MoveType::Toss;
        entity.solid = Q1Solid::Trigger;
        entity.movement_flags = 256 | 131072;
    })?;
    let origin = ctf_body(game, &id)?.origin;
    set_addon_number(game, &flag, "ctf.return", game.time + 15.0)?;
    game.set_body(
        &flag,
        &BodyPatch {
            origin: Some(vsub(
                origin,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 24.0,
                },
            )),
            velocity: Some(Vec3 {
                x: 0.0,
                y: 0.0,
                z: 300.0,
            }),
            bounds: Some(CTF_FLAG_BOUNDS),
            ..Default::default()
        },
    )?;
    game.link(&flag)?;
    ctf_update(game, None)
}

/// Handle a flag touch: pickup, own-flag return, or capture
/// (`touchFlag`).
pub fn touch_flag(game: &mut Q1EntityServices, flag: &ActorId, actor: &ActorId) -> Result<(), Q1Error> {
    let flag = flag.clone();
    let id = actor.clone();
    let solid = game
        .entity_ref(&flag)
        .map(|entity| entity.solid)
        .unwrap_or(Q1Solid::None);
    if solid != Q1Solid::Trigger || !game.is_player(&id) || game.health(&id) <= 0.0 {
        return Ok(());
    }
    if with_ctf_services(game, |services| services.observer(&id))? {
        return Ok(());
    }
    let team = ctf_team(game, &id);
    let Some(team) = team else {
        return Ok(());
    };
    if Some(team) != ctf_last_team(game, &id)? {
        return Ok(());
    }
    let own = ctf_flag_team(game, &flag);
    if team == own {
        let count = game.entity_ref(&flag).map(|entity| entity.count).unwrap_or(0.0);
        if count == 0.0 {
            if ctf_carried(game, &id).is_none() {
                return Ok(());
            }
            ctf_announce(
                game,
                if team == CtfTeam::Red {
                    "$qc_ks_blue_captured"
                } else {
                    "$qc_ks_red_captured"
                },
                Some(&id),
                "",
            )?;
            with_ctf_services(game, |services| services.log(&id, "FLAG-CAPTURE"))?;
            ctf_grant(game, &id, "q1:key/silver", 0.0, 1.0)?;
            ctf_grant(game, &id, "q1:key/gold", 0.0, 1.0)?;
            let world = ctf_world(game)?;
            set_addon_number(game, &world, "ctf.lastCapture", game.time)?;
            set_addon_number(game, &world, "ctf.lastCaptureTeam", team_number(Some(team)) as f64)?;
            let owner = ctf_owner(game, &id)?;
            game.sound(owner.id(), "misc/flagcap.wav", Q1SoundChannel::Voice, 0.0, 1.0)?;
            with_ctf_services(game, |services| services.add_capture(team))?;
            with_ctf_services(game, |services| services.add_score(&id, 15.0))?;
            for player in (game.host.players)() {
                ctf_set(game, &player, "killed", 0.0)?;
                if ctf_last_team(game, &player)? == Some(team) {
                    if !same_actor(&player, &id) {
                        with_ctf_services(game, |services| services.add_score(&player, 10.0))?;
                    }
                    if ctf_number(game, &player, "lastReturned")? + 4.0 > game.time {
                        ctf_announce(game, "$qc_ks_assist", Some(&player), "")?;
                        with_ctf_services(game, |services| services.add_score(&player, 1.0))?;
                    }
                    if ctf_number(game, &player, "lastFraggedCarrier")? + 6.0 > game.time {
                        ctf_announce(game, "$qc_ks_assist_carrier", Some(&player), "")?;
                        with_ctf_services(game, |services| services.add_score(&player, 2.0))?;
                    }
                } else {
                    ctf_set(game, &player, "lastHurtCarrier", -5.0)?;
                }
                game.message(
                    Some(&player),
                    if ctf_last_team(game, &player)? == Some(team) {
                        "$qc_ctf_team_captured"
                    } else {
                        "$qc_ctf_your_captured"
                    },
                    true,
                    Vec::new(),
                );
            }
            for color in [team, opposite(team)] {
                if let Some(home) = ctf_flag(game, color) {
                    return_flag(game, &home, false)?;
                }
            }
        } else {
            ctf_announce(
                game,
                if team == CtfTeam::Red {
                    "$qc_ks_red_returned"
                } else {
                    "$qc_ks_blue_returned"
                },
                Some(&id),
                "",
            )?;
            with_ctf_services(game, |services| services.log(&id, "FLAG-RECOVERY"))?;
            with_ctf_services(game, |services| services.add_score(&id, 1.0))?;
            ctf_set(game, &id, "lastReturned", game.time)?;
            let owner = ctf_owner(game, &id)?;
            game.sound(owner.id(), "doors/runetry.wav", Q1SoundChannel::Item, 1.0, 1.0)?;
            return_flag(game, &flag, true)?;
        }
        return ctf_update(game, None);
    }
    ctf_announce(game, "$qc_ks_blue_picked_up", Some(&id), "")?;
    with_ctf_services(game, |services| services.log(&id, "FLAG-PICKUP"))?;
    game.message(Some(&id), "$qc_ctf_have_flag", true, Vec::new());
    let owner = ctf_owner(game, &id)?;
    game.sound(owner.id(), "misc/flagtk.wav", Q1SoundChannel::Item, 1.0, 1.0)?;
    ctf_grant(
        game,
        &id,
        if own == CtfTeam::Red {
            "q1:key/gold"
        } else {
            "q1:key/silver"
        },
        1.0,
        1.0,
    )?;
    ctf_set(game, &id, "flagSince", game.time)?;
    game.update_entity(&flag, |entity| {
        entity.count = 1.0;
        entity.movement = Q1MoveType::Noclip;
        entity.solid = Q1Solid::None;
        entity.owner = Some(id.clone());
    })?;
    game.link(&flag)?;
    for player in (game.host.players)() {
        if same_actor(&player, &id) {
            continue;
        }
        game.message(
            Some(&player),
            if ctf_team(game, &player) == Some(team) {
                "$qc_ctf_your_has"
            } else {
                "$qc_ctf_your_taken"
            },
            true,
            Vec::new(),
        );
    }
    ctf_update(game, None)
}

/// Track carriers, time out dropped flags, and validate flag state
/// (`flagThink`).
fn flag_think(game: &mut Q1EntityServices, flag: &ActorId) -> Result<(), Q1Error> {
    let flag = flag.clone();
    game.schedule(&flag, 0.1, "ctf:flag_think")?;
    let count = game.entity_ref(&flag).map(|entity| entity.count).unwrap_or(0.0);
    if count == 0.0 {
        return Ok(());
    }
    if count == 2.0 {
        let returned = game
            .entity_ref(&flag)
            .map(|entity| entity.number("ctf.return"))
            .unwrap_or(0.0);
        // QC stores time+15 at drop, then compares time-super_time>15:
        // the authored delay is 30 seconds.
        if game.time - returned > 15.0 {
            return_flag(game, &flag, true)?;
        }
        return ctf_update(game, None);
    }
    if count != 1.0 {
        return Err(q1_error("CTF flag has an invalid source state"));
    }
    let carrier = game.entity_ref(&flag).and_then(|entity| entity.owner.clone());
    let Some(carrier) = carrier else {
        return return_flag(game, &flag, true);
    };
    if !game.host.actors.is_live(&carrier) {
        return return_flag(game, &flag, true);
    }
    if game.health(&carrier) <= 0.0 {
        return drop_flag(game, &carrier);
    }
    let body = ctf_body(game, &carrier)?;
    let frame = with_ctf_services(game, |services| services.input(&carrier).frame)?;
    let basis = vectors(body.angles);
    let mut distance = 14.0;
    if (29..=34).contains(&frame) {
        distance += [2.0, 8.0, 12.0, 11.0, 10.0, 4.0][(frame - 29) as usize];
    } else if (35..=40).contains(&frame) {
        distance += [2.0, 10.0, 10.0, 8.0, 4.0, 2.0][(frame - 35) as usize];
    } else if (103..=118).contains(&frame) {
        distance += if frame <= 106 { 6.0 } else { 7.0 };
    }
    let forward = Vec3 {
        x: basis.forward.x,
        y: basis.forward.y,
        z: -basis.forward.z,
    };
    let origin = vadd(
        vsub(
            vadd(
                body.origin,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: -16.0,
                },
            ),
            vscale(forward, distance),
        ),
        vscale(basis.right, 22.0),
    );
    game.set_body(
        &flag,
        &BodyPatch {
            origin: Some(origin),
            angles: Some(vadd(
                body.angles,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: -45.0,
                },
            )),
            ..Default::default()
        },
    )?;
    game.link(&flag)?;
    game.schedule(&flag, 0.01, "ctf:flag_think")
}

fn flag_touch(
    game: &mut Q1EntityServices,
    flag: &ActorId,
    actor: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    touch_flag(game, flag, actor)
}

fn place_flag(game: &mut Q1EntityServices, flag: &ActorId) -> Result<(), Q1Error> {
    let flag = flag.clone();
    let body = game.body(&flag)?;
    let start = vadd(body.origin, Vec3 { x: 0.0, y: 0.0, z: 6.0 });
    let floor = game.host.trace(&Q1TraceRequest {
        start,
        end: vsub(
            start,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 256.0,
            },
        ),
        bounds: body.bounds,
        ignore: Some(flag.clone()),
        monsters: true,
        missile: false,
    });
    if floor.all_solid || floor.fraction == 1.0 {
        return game.remove(&flag);
    }
    let touch = game.named.touch("ctf:flag_touch")?;
    let angles = body.angles;
    game.update_entity(&flag, |entity| {
        entity.solid = Q1Solid::Trigger;
        entity.movement = Q1MoveType::Toss;
        entity.movement_flags = 256 | 131072;
        entity.count = 0.0;
        entity.mangle = angles;
        entity.effects |= 8;
        entity.touch = Some(touch);
    })?;
    crate::q1::addons::ctf::state::ctf_set_entity_vector(game, &flag, "ctf.base", floor.end)?;
    game.set_body(
        &flag,
        &BodyPatch {
            origin: Some(floor.end),
            velocity: Some(ZERO),
            ground: Some(floor.actor.clone()),
            ..Default::default()
        },
    )?;
    game.link(&flag)?;
    game.schedule(&flag, 0.1, "ctf:flag_think")
}

fn spawn_flag_team(game: &mut Q1EntityServices, flag: &ActorId, skin: i32) -> Result<(), Q1Error> {
    game.update_entity(flag, |entity| {
        entity.model = String::from("progs/flag.mdl");
        entity.skin = skin;
        entity.effects = if skin == 0 { 32 } else { 16 };
    })?;
    game.set_bounds(flag, CTF_FLAG_BOUNDS)?;
    game.schedule(flag, 0.2, "ctf:place_flag")
}

fn spawn_flag_team1(game: &mut Q1EntityServices, flag: &ActorId) -> Result<(), Q1Error> {
    spawn_flag_team(game, flag, 0)
}

fn spawn_flag_team2(game: &mut Q1EntityServices, flag: &ActorId) -> Result<(), Q1Error> {
    spawn_flag_team(game, flag, 1)
}

/// Register flag callbacks and spawn handlers (`registerFlags`).
pub fn register_flags(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        "ctf:flag_touch",
        Q1CallbackHandlers {
            touch: Some(flag_touch as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        "ctf:flag_think",
        Q1CallbackHandlers {
            action: Some(flag_think as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        "ctf:place_flag",
        Q1CallbackHandlers {
            action: Some(place_flag as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.register_spawn("item_flag_team1", spawn_flag_team1)?;
    game.register_spawn("item_flag_team2", spawn_flag_team2)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::{attach_test_player, register_test_addons, Q1AddonProgram};
    use crate::q1::addons::ctf::state::register_ctf_state;
    use crate::q1::addons::ctf::teams::set_team;
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

    fn make_flag(game: &mut Q1EntityServices, team: CtfTeam) -> ActorId {
        let classname = match team {
            CtfTeam::Red => "item_flag_team1",
            CtfTeam::Blue => "item_flag_team2",
        };
        let flag = game.create(classname, None, None).expect("flag");
        game.update_entity(&flag, |entity| {
            entity.solid = Q1Solid::Trigger;
            entity.movement = Q1MoveType::Toss;
            entity.count = 0.0;
        })
        .expect("flag state");
        game.set_body(
            &flag,
            &BodyPatch {
                origin: Some(ZERO),
                ..Default::default()
            },
        )
        .expect("flag body");
        flag
    }

    #[test]
    fn enemy_pickup_attaches_and_drop_releases() {
        let mut game = test_game();
        let (_guard, player) = setup(&mut game);
        register_flags(&mut game).expect("flags");
        set_team(&mut game, &player, Some(CtfTeam::Blue)).expect("team");
        let flag = make_flag(&mut game, CtfTeam::Red);
        touch_flag(&mut game, &flag, &player).expect("touch");
        assert_eq!(ctf_carried(&game, &player), Some(flag.clone()));
        assert_eq!(game.entity_ref(&flag).map(|entity| entity.count), Some(1.0));
        drop_flag(&mut game, &player).expect("drop");
        assert_eq!(ctf_carried(&game, &player), None);
        assert_eq!(game.entity_ref(&flag).map(|entity| entity.count), Some(2.0));
    }

    #[test]
    fn own_dropped_flag_returns_on_touch() {
        let mut game = test_game();
        let (_guard, player) = setup(&mut game);
        register_flags(&mut game).expect("flags");
        set_team(&mut game, &player, Some(CtfTeam::Red)).expect("team");
        let flag = make_flag(&mut game, CtfTeam::Red);
        game.update_entity(&flag, |entity| {
            entity.count = 2.0;
        })
        .expect("dropped");
        touch_flag(&mut game, &flag, &player).expect("touch");
        assert_eq!(game.entity_ref(&flag).map(|entity| entity.count), Some(0.0));
    }
}
