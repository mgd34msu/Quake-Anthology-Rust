//! Hipnotic Armagon (`src/content/q1/missionpacks/monsters/armagon.ts`).

use std::sync::Arc;

use qa_core::identity::{same_actor, ActorId};
use qa_core::math::{Bounds, Vec3};

use crate::q1::base::animation::MonsterAi;
use crate::q1::base::projectiles::{create_missile, throw_gib};
use crate::q1::base::species::{MonsterMovement, MonsterSpecies};
use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity::{Q1MonsterSpecies, Q1ProjectileKind};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::BodyPatch;
use crate::q1::foundation::types::{
    dot, length, normalize, vadd, vscale, vsub, yaw_for, Q1Edition, Q1Event, Q1MoveType, Q1Solid, Q1SoundChannel,
    Q1TraceRequest, POINT, ZERO,
};
use crate::q1::missionpacks::hipnotic_weapons::launch_hipnotic_laser;
use crate::q1::missionpacks::types::Q1MissionPack;
use crate::q1::missionpacks::world::hipnotic_misc::multi_explosion;
use crate::q1::Q1Error;

use super::helpers::{eye, number, radius_actors};
use super::runtime::{MissionMonster, Q1MissionPackMonsters};
use super::tables::hiparma::FRAMES;
use super::types::{MissionAction, MissionCheckAttack, MissionDie, MissionFound, MissionPain, PackMonsterDefinition};

/// Armagon's separate body entity (`bodyPart`).
fn body_part(monster: &MissionMonster) -> ActorId {
    monster
        .entity
        .references
        .get("trigger_field")
        .cloned()
        .flatten()
        .expect("Armagon lost its source body entity")
}

/// Emit an Armagon sound (`sound`).
fn sound(monster: &mut MissionMonster, path: &str, channel: Q1SoundChannel, volume: f64, attenuation: f64) {
    monster.game.host.emit(Q1Event::Sound {
        origin: None,
        actor: monster.entity.actor.id().clone(),
        path: path.to_string(),
        channel,
        attenuation,
        volume,
    });
}

/// Enemy ground position (`enemyOrigin`).
fn enemy_origin(monster: &MissionMonster) -> Vec3 {
    monster.target.unwrap_or(ZERO)
}

/// Enemy eye position (`enemyEye`).
fn enemy_eye(monster: &mut MissionMonster) -> Vec3 {
    match monster.enemy.clone() {
        Some(enemy) => eye(monster.game, &enemy).unwrap_or(ZERO),
        None => ZERO,
    }
}

/// Turn the upper body toward the target (`turn`).
fn turn(monster: &mut MissionMonster, difference: f64, target: f64) {
    let current = monster.entity.number("fixangle");
    number(
        monster,
        "fixangle",
        if difference.abs() < 10.0 {
            target
        } else if difference > 5.0 {
            current + 9.0
        } else if difference < -5.0 {
            current - 9.0
        } else {
            target
        },
    );
}

/// Sync the body entity to the legs (`syncBody`).
fn sync_body(monster: &mut MissionMonster, walking: bool) -> ActorId {
    let id = monster.entity.actor.id().clone();
    let body = body_part(monster);
    let origin = monster.origin;
    let rerelease = monster.game.options().edition == Q1Edition::Rerelease;
    let _ = monster.game.set_origin(&body, origin);
    if walking && rerelease {
        let _ = monster.game.update_entity(&body, |entity| {
            entity.movement = Q1MoveType::Step;
        });
    }
    let frame = monster.entity.frame;
    let _ = monster.game.update_entity(&body, |entity| {
        entity.frame = frame;
    });
    if let Ok(legs) = monster.game.body(&id) {
        let fixangle = monster.entity.number("fixangle");
        let _ = monster.game.set_body(
            &body,
            &BodyPatch {
                angles: Some(Vec3 {
                    x: legs.angles.x,
                    y: legs.angles.y + fixangle as f32,
                    z: legs.angles.z,
                }),
                ..Default::default()
            },
        );
    }
    monster.refresh();
    body
}

/// Play an idle sound (`idleSound`).
fn idle_sound(monster: &mut MissionMonster) {
    let id = monster.entity.actor.id().clone();
    if monster.game.health(&id) < 0.0 || monster.game.time <= monster.entity.number("super_time") {
        return;
    }
    let time = monster.game.time;
    number(monster, "super_time", time + 3.0);
    if monster.game.host.random() < 0.5 {
        let rolled = monster.game.host.random();
        let path = if rolled < 0.25 {
            "armagon/idle1.wav"
        } else if rolled < 0.5 {
            "armagon/idle2.wav"
        } else if rolled < 0.75 {
            "armagon/idle3.wav"
        } else {
            "armagon/idle4.wav"
        };
        sound(monster, path, Q1SoundChannel::Voice, 1.0, 0.5);
    }
}

/// Face the enemy and sync the body (`think`).
fn think(monster: &mut MissionMonster) {
    let id = monster.entity.actor.id().clone();
    let body = sync_body(monster, false);
    monster.entity.ideal_yaw = yaw_for(vsub(enemy_origin(monster), monster.origin));
    monster.flush_entity();
    let mut delta = monster.entity.ideal_yaw
        - monster
            .game
            .body(&id)
            .map(|body| f64::from(body.angles.y))
            .unwrap_or(0.0);
    number(monster, "cnt", 0.0);
    if delta > 180.0 {
        delta -= 360.0;
    }
    if delta < -180.0 {
        delta += 360.0;
    }
    if delta.abs() > 90.0 {
        delta = 0.0;
        number(monster, "cnt", 1.0);
    }
    let fixangle = monster.entity.number("fixangle");
    turn(monster, delta - fixangle, delta);
    if monster.game.health(&id) < 0.0 {
        return;
    }
    idle_sound(monster);
    if monster.game.options().edition == Q1Edition::Rerelease {
        let moved = monster.game.entity(&body).map(|entity| entity.vector("oldorigin"));
        let at = monster.game.body(&body).map(|body| body.origin).ok();
        if let (Some(was), Some(at)) = (moved, at) {
            if f64::from(length(vsub(was, at))) > 50.0 {
                let _ = monster.game.update_entity(&body, |entity| {
                    entity.movement = Q1MoveType::Step;
                });
            }
        }
    }
    monster.refresh();
}

/// Walk while scanning for clients (`walkThink`).
fn walk_think(monster: &mut MissionMonster) {
    sync_body(monster, true);
    monster.change_yaw();
    number(monster, "cnt", 0.0);
    let fixangle = monster.entity.number("fixangle");
    turn(monster, -fixangle, 0.0);
    let id = monster.entity.actor.id().clone();
    if monster.game.health(&id) < 0.0 {
        return;
    }
    idle_sound(monster);
    let owned = monster.entity.actor.clone();
    if let Some(client) = monster.game.host.check_client(&owned) {
        if monster.visible(Some(&client)) {
            monster.found(&client);
        }
    }
}

/// Fire a gun or laser burst (`launch`).
fn launch(monster: &mut MissionMonster, offset: f64, turn_direction: i32, laser: bool) {
    monster.sync();
    let id = monster.entity.actor.id().clone();
    let Ok(legs) = monster.game.body(&id) else {
        monster.refresh();
        return;
    };
    let fixangle = monster.entity.number("fixangle");
    let twist = if turn_direction == 1 {
        165.0
    } else if turn_direction == 2 {
        -165.0
    } else {
        0.0
    };
    let basis = monster.game.make_vectors(Vec3 {
        x: legs.angles.x,
        y: legs.angles.y + (fixangle + twist) as f32,
        z: legs.angles.z,
    });
    let origin = vadd(
        vadd(
            vadd(
                monster.origin,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 66.0,
                },
            ),
            vscale(basis.right, offset),
        ),
        vscale(basis.forward, 84.0),
    );
    let mut target = enemy_eye(monster);
    if monster.game.options().skill != 0 {
        let velocity = monster
            .enemy
            .clone()
            .and_then(|enemy| monster.game.host.bodies.read(&enemy))
            .map(|body| body.velocity)
            .unwrap_or(ZERO);
        target = vadd(
            target,
            vscale(velocity, f64::from(length(vsub(target, origin))) / 1000.0),
        );
    }
    let mut direction = normalize(vsub(target, origin));
    if f64::from(dot(direction, basis.forward)) < monster.entity.number("worldtype") {
        direction = basis.forward;
    }
    monster.entity.effects |= 2;
    monster.flush_entity();
    if laser {
        let _ = launch_hipnotic_laser(monster.game, &id, origin, direction, false, None);
    } else {
        let _ = monster
            .game
            .sound(&id, "weapons/sgun1.wav", Q1SoundChannel::Weapon, 1.0, 1.0);
        let punch = monster.entity.vector("punchangle");
        monster
            .entity
            .fields
            .insert("punchangle".to_string(), format!("-2 {} {}", punch.y, punch.z));
        monster.flush_entity();
        if let Ok(shot) = create_missile(
            monster.game,
            Some(&id),
            "missile",
            "missile",
            origin,
            vscale(direction, 1000.0),
            5.0,
        ) {
            if let Ok(touch) = monster.game.named.touch("projectile_touch") {
                let _ = monster.game.update_entity(&shot, |entity| {
                    entity.projectile = Some(Q1ProjectileKind::Rocket);
                    entity.touch = Some(touch);
                });
            }
        }
    }
    monster.refresh();
}

/// Strafe around the enemy while firing (`overThink`).
fn over_think(monster: &mut MissionMonster, left: bool) {
    monster.change_yaw();
    let id = monster.entity.actor.id().clone();
    let owned = monster.entity.actor.clone();
    let yaw = monster
        .game
        .body(&id)
        .map(|body| f64::from(body.angles.y))
        .unwrap_or(0.0);
    monster.game.host.walk_move(&owned, yaw, 14.0);
    sync_body(monster, false);
    let mut delta = 0.0;
    if monster.entity.count == 0.0 {
        monster.entity.ideal_yaw = yaw_for(vsub(enemy_origin(monster), monster.origin));
        monster.flush_entity();
        delta = monster.entity.ideal_yaw - yaw + if left { -165.0 } else { 165.0 };
        if delta > 180.0 {
            delta -= 360.0;
        }
        if delta < -180.0 {
            delta += 360.0;
        }
    } else if monster.entity.count == 1.0 {
        launch(
            monster,
            if left { 40.0 } else { -40.0 },
            if left { 1 } else { 2 },
            false,
        );
        return;
    }
    let fixangle = monster.entity.number("fixangle");
    turn(monster, delta - fixangle, delta);
}

/// Walk toward the path goal (`walk`).
fn walk(monster: &mut MissionMonster) {
    let path = monster.state.path.clone();
    let goal = monster
        .game
        .find(&path)
        .first()
        .cloned()
        .or_else(|| monster.game.world.clone());
    if let Some(goal) = goal {
        let owned = monster.entity.actor.clone();
        monster.game.host.move_to_goal(&owned, &goal, 14.0, None);
    }
    monster.refresh();
}

/// Run straight at the enemy (`run`).
fn run(monster: &mut MissionMonster) {
    let Some(world) = monster.game.world.clone() else {
        panic!("Armagon requires a world entity");
    };
    monster.change_yaw();
    let _ = monster.game.update_entity(&world, |entity| {
        entity.fields.insert("RUN_STRAIGHT".to_string(), "1".to_string());
    });
    monster.ai(MonsterAi::Run, 14.0);
    think(monster);
}

/// Attack while walking (`walkingAttack`).
fn walking_attack(monster: &mut MissionMonster) {
    monster.change_yaw();
    let id = monster.entity.actor.id().clone();
    let owned = monster.entity.actor.clone();
    let yaw = monster
        .game
        .body(&id)
        .map(|body| f64::from(body.angles.y))
        .unwrap_or(0.0);
    monster.game.host.walk_move(&owned, yaw, 14.0);
    think(monster);
}

/// Repulse nearby players (`repulse`).
fn repulse(monster: &mut MissionMonster) {
    think(monster);
    let id = monster.entity.actor.id().clone();
    if monster.entity.number("state") == 0.0 {
        monster.attack_finished(0.5);
        let _ = monster
            .game
            .sound(&id, "armagon/repel.wav", Q1SoundChannel::Body, 1.0, 1.0);
        number(monster, "state", 1.0);
        return;
    }
    if monster.entity.number("state") != 1.0 {
        return;
    }
    for actor in radius_actors(monster.game, monster.origin, 300.0) {
        let flags = monster
            .game
            .entity(&actor)
            .map(|entity| entity.movement_flags)
            .unwrap_or(0);
        if !monster.game.is_player(&actor)
            || flags & 128 != 0
            || !monster.visible(Some(&actor))
            || monster.game.health(&actor) <= 0.0
        {
            continue;
        }
        let owned = monster.game.host.actors.resolve_owned(&actor);
        let body = monster.game.host.bodies.read(&actor);
        let (Some(owned), Some(body)) = (owned, body) else {
            continue;
        };
        let direction = normalize(vsub(
            body.origin,
            vsub(
                monster.origin,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 24.0,
                },
            ),
        ));
        let mut moved = body.clone();
        moved.velocity = vadd(body.velocity, vscale(direction, 1500.0));
        let _ = monster.game.host.bodies.write(&owned, &moved);
    }
    monster.game.radius_damage(&id, Some(&id), 60.0, Some(&id), None, "");
    number(monster, "state", 0.0);
    monster.attack_finished(0.1);
}

/// Line of fire to the enemy (`clearShot`).
fn clear_shot(monster: &mut MissionMonster) -> (bool, bool, f64) {
    let id = monster.entity.actor.id().clone();
    let start = monster.eye(None).unwrap_or(monster.origin);
    let end = enemy_eye(monster);
    let enemy = monster.enemy.clone().or_else(|| monster.game.world.clone());
    let trace = monster.game.host.trace(&Q1TraceRequest {
        start,
        end,
        bounds: POINT,
        ignore: Some(id),
        monsters: true,
        missile: false,
    });
    let clear = enemy
        .as_ref()
        .is_some_and(|enemy| trace.actor.as_ref().is_some_and(|hit| same_actor(hit, enemy)));
    (
        clear,
        trace.in_open && trace.in_water,
        f64::from(length(vsub(end, start))),
    )
}

/// Attack from standing (`standAttack`).
fn stand_attack(monster: &mut MissionMonster) {
    let (clear, crossed_water, distance) = clear_shot(monster);
    if !clear || crossed_water {
        monster.play("armagon_run1");
        return;
    }
    if monster.game.time < monster.state.attack_finished {
        return;
    }
    if distance < 200.0
        && monster
            .enemy
            .clone()
            .is_some_and(|enemy| monster.game.is_player(&enemy))
    {
        repulse(monster);
        return;
    }
    number(monster, "state", 0.0);
    if distance > 450.0 {
        monster.play("armagon_run1");
        return;
    }
    let frame = if monster.game.host.random() < 0.5 {
        "armagon_satk1"
    } else {
        "armagon_slaser1"
    };
    monster.play(frame);
    if monster.entity.number("cnt") == 1.0 {
        monster.play("armagon_run1");
    }
}

/// Check for a standing attack (`checkAttack`).
fn check_attack(monster: &mut MissionMonster) -> bool {
    monster.lefty = false;
    let (clear, crossed_water, distance) = clear_shot(monster);
    let id = monster.entity.actor.id().clone();
    if (!clear && monster.entity.number("charmed") == 0.0)
        || crossed_water
        || monster.game.time < monster.state.attack_finished
    {
        return false;
    }
    let delta = monster.entity.ideal_yaw
        - (monster
            .game
            .body(&id)
            .map(|body| f64::from(body.angles.y))
            .unwrap_or(0.0)
            + monster.entity.number("fixangle"));
    if (delta.abs() > 10.0 && distance > 200.0)
        || monster.enemy.is_none()
        || monster
            .enemy
            .clone()
            .is_some_and(|enemy| !monster.game.is_player(&enemy))
    {
        return false;
    }
    if distance < 400.0 {
        monster.play("armagon_stop1");
        return true;
    }
    monster.lefty = true;
    false
}

/// Staged body explosion ticks (`armagon_body_explode1`).
fn body_explode(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let again = game.named.action("hipnotic:armagon_body_explode1")?;
    game.schedule(&id, 0.1, &again)?;
    let count = game.entity(&id).map(|entity| entity.number("cnt")).unwrap_or(0.0);
    if count == 0.0 {
        game.update_entity(&id, |entity| entity.count = 0.0)?;
    }
    if count < 25.0 {
        let tracked = game.entity(&id).map(|entity| entity.count).unwrap_or(0.0);
        if count > tracked {
            let origin = game.body(&id).map(|body| body.origin).unwrap_or(ZERO);
            for model in ["gib1", "gib2", "gib3"] {
                let _ = throw_gib(game, origin, model, -100.0);
            }
            game.update_entity(&id, |entity| entity.count = count + 1.0)?;
        }
        game.update_entity(&id, |entity| {
            entity.fields.insert("cnt".to_string(), (count + 1.0).to_string());
        })?;
    } else {
        game.update_entity(&id, |entity| {
            entity.fields.insert("cnt".to_string(), "0".to_string());
        })?;
        let finish = game.named.action("hipnotic:armagon_body_explode2")?;
        game.schedule(&id, 0.1, &finish)?;
    }
    Ok(())
}

/// Final body explosion (`armagon_body_explode2`).
fn body_explosion(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    game.sound(&id, "misc/longexpl.wav", Q1SoundChannel::Auto, 0.5, 1.0)?;
    for _ in 0..3 {
        let origin = game.body(&id).map(|body| body.origin).unwrap_or(ZERO);
        for model in ["gib1", "gib2", "gib3"] {
            let _ = throw_gib(game, origin, model, -200.0);
        }
    }
    game.update_entity(&id, |entity| {
        entity.movement = Q1MoveType::None;
        entity.model = "progs/s_explod.spr".to_string();
        entity.solid = Q1Solid::None;
        entity.frame = 0;
    })?;
    let frame = game.named.action("base:explosion_frame")?;
    game.schedule(&id, 0.1, &frame)
}

/// Final death sequence (`finalDeath`).
fn final_death(monster: &mut MissionMonster) {
    think(monster);
    let id = monster.entity.actor.id().clone();
    let origin = vadd(
        monster.origin,
        Vec3 {
            x: 0.0,
            y: 0.0,
            z: 80.0,
        },
    );
    let _ = multi_explosion(monster.game, &id, origin, 20.0, 10.0, 3.0, 0.1, 0.5);
    monster.game.cancel(&id);
    monster.entity.movement = Q1MoveType::None;
    monster.entity.solid = Q1Solid::None;
    monster.entity.movement_flags = 0;
    let time = monster.game.time;
    monster.entity.wait = time + 5.0;
    monster.entity.fields.insert("gorging".to_string(), "1".to_string());
    monster.flush_entity();
    let _ = monster.game.set_damageable(&id, false);
    let _ = monster.game.set_bounds(
        &id,
        Bounds {
            min: Vec3 {
                x: -32.0,
                y: -32.0,
                z: -24.0,
            },
            max: Vec3 {
                x: 32.0,
                y: 32.0,
                z: 32.0,
            },
        },
    );
    let body = body_part(monster);
    monster.game.cancel(&body);
    let _ = monster.game.update_entity(&body, |entity| {
        entity.solid = Q1Solid::None;
        entity.fields.insert("gorging".to_string(), "1".to_string());
    });
    let _ = monster.game.set_damageable(&body, false);
    if let Ok(explode) = monster.game.named.action("hipnotic:armagon_body_explode1") {
        let _ = monster.game.schedule(&body, 0.1, &explode);
    }
    monster.refresh();
}

/// Hipnotic Armagon definition (`armagonDefinition`). The donor reads the
/// skill through the runtime's game handle; this port takes the game
/// directly since the runtime carries no services.
pub fn armagon_definition(game: &Q1EntityServices, runtime: &Q1MissionPackMonsters) -> PackMonsterDefinition {
    debug_assert!(
        matches!(runtime.pack, Q1MissionPack::Hipnotic),
        "armagon is Hipnotic-only"
    );
    let skill = game.options().skill;
    let servo: MissionAction = Arc::new(|monster| {
        sound(monster, "armagon/servo.wav", Q1SoundChannel::Raw(7), 0.5, 0.5);
    });
    let foot: MissionAction = Arc::new(|monster| {
        sound(monster, "armagon/footfall.wav", Q1SoundChannel::Raw(6), 1.0, 0.5);
    });
    let mut actions: Vec<(&'static str, MissionAction)> = vec![
        ("armagon_think", Arc::new(think)),
        ("armagon_walkthink", Arc::new(walk_think)),
        ("armagon_overleft_think", Arc::new(|monster| over_think(monster, true))),
        (
            "armagon_overright_think",
            Arc::new(|monster| over_think(monster, false)),
        ),
        ("armagon_stand_attack", Arc::new(stand_attack)),
        (
            "armagon_missile_attack",
            Arc::new(|monster| {
                let frame = if monster.game.host.random() < 0.5 {
                    "armagon_watk1"
                } else {
                    "armagon_wlaseratk1"
                };
                monster.play(frame);
            }),
        ),
        ("movetogoal(14)", Arc::new(walk)),
        (
            "hiparma:armagon_stand1",
            Arc::new(|monster| {
                monster.ai(MonsterAi::Stand, 0.0);
                think(monster);
                monster.delay(0.2);
            }),
        ),
        (
            "hiparma:armagon_stand2",
            Arc::new(|monster| {
                think(monster);
                monster.delay(0.2);
            }),
        ),
        (
            "hiparma:armagon_walk3",
            Arc::new({
                let servo = servo.clone();
                move |monster: &mut MissionMonster| {
                    servo(monster);
                    walk(monster);
                    walk_think(monster);
                }
            }),
        ),
        (
            "hiparma:armagon_walk5",
            Arc::new({
                let foot = foot.clone();
                move |monster: &mut MissionMonster| {
                    foot(monster);
                    walk(monster);
                    walk_think(monster);
                }
            }),
        ),
        ("hiparma:armagon_run1", Arc::new(run)),
        (
            "hiparma:armagon_run3",
            Arc::new({
                let servo = servo.clone();
                move |monster: &mut MissionMonster| {
                    servo(monster);
                    run(monster);
                }
            }),
        ),
        (
            "hiparma:armagon_run5",
            Arc::new({
                let foot = foot.clone();
                move |monster: &mut MissionMonster| {
                    foot(monster);
                    run(monster);
                }
            }),
        ),
        (
            "hiparma:armagon_run12",
            Arc::new(|monster| {
                run(monster);
                let id = monster.entity.actor.id().clone();
                if monster.entity.number("cnt") == 1.0 && monster.game.time > monster.state.attack_finished {
                    let mut delta = monster.entity.ideal_yaw
                        - monster
                            .game
                            .body(&id)
                            .map(|body| f64::from(body.angles.y))
                            .unwrap_or(0.0);
                    if delta > 180.0 {
                        delta -= 360.0;
                    }
                    if delta < -180.0 {
                        delta += 360.0;
                    }
                    monster.next_frame = if delta > 0.0 {
                        "armagon_overleft1"
                    } else {
                        "armagon_overright1"
                    }
                    .to_string();
                } else if monster.lefty {
                    monster.lefty = false;
                    monster.next_frame = "armagon_missile_attack".to_string();
                }
            }),
        ),
        ("hiparma:armagon_watk1", Arc::new(walking_attack)),
        (
            "hiparma:armagon_watk2",
            Arc::new({
                let servo = servo.clone();
                move |monster: &mut MissionMonster| {
                    servo(monster);
                    walking_attack(monster);
                }
            }),
        ),
        (
            "hiparma:armagon_watk4",
            Arc::new({
                let foot = foot.clone();
                move |monster: &mut MissionMonster| {
                    foot(monster);
                    walking_attack(monster);
                }
            }),
        ),
        (
            "hiparma:armagon_watk6",
            Arc::new(|monster| {
                walking_attack(monster);
                launch(monster, 40.0, 0, false);
            }),
        ),
        (
            "hiparma:armagon_watk11",
            Arc::new(|monster| {
                walking_attack(monster);
                launch(monster, -40.0, 0, false);
            }),
        ),
        (
            "hiparma:armagon_watk13",
            Arc::new(|monster| {
                walking_attack(monster);
                monster.attack_finished(1.0);
            }),
        ),
        (
            "hiparma:armagon_wlaseratk6",
            Arc::new(|monster| {
                walking_attack(monster);
                launch(monster, 40.0, 0, true);
            }),
        ),
        (
            "hiparma:armagon_wlaseratk11",
            Arc::new(|monster| {
                walking_attack(monster);
                launch(monster, -40.0, 0, true);
            }),
        ),
        (
            "SUB_AttackFinished(1.0)",
            Arc::new(|monster| monster.attack_finished(1.0)),
        ),
        (
            "SUB_AttackFinished(0.3)",
            Arc::new(|monster| monster.attack_finished(0.3)),
        ),
        (
            "hiparma:armagon_stop2",
            Arc::new({
                let foot = foot.clone();
                move |monster: &mut MissionMonster| {
                    foot(monster);
                    think(monster);
                }
            }),
        ),
        (
            "hiparma:armagon_satk6",
            Arc::new({
                let foot = foot.clone();
                move |monster: &mut MissionMonster| {
                    foot(monster);
                    think(monster);
                }
            }),
        ),
        (
            "hiparma:armagon_satk9",
            Arc::new(|monster| {
                think(monster);
                launch(monster, 40.0, 0, false);
                launch(monster, -40.0, 0, false);
            }),
        ),
        (
            "armagon_launch_laser(40)",
            Arc::new(|monster| launch(monster, 40.0, 0, true)),
        ),
        (
            "armagon_launch_laser(-40)",
            Arc::new(|monster| launch(monster, -40.0, 0, true)),
        ),
        (
            "hiparma:armagon_die4",
            Arc::new(|monster| {
                think(monster);
                let id = monster.entity.actor.id().clone();
                let origin = vadd(
                    monster.origin,
                    Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 48.0,
                    },
                );
                let _ = multi_explosion(monster.game, &id, origin, 48.0, 10.0, 6.0, 0.3, 0.3);
                sound(monster, "armagon/death.wav", Q1SoundChannel::Auto, 1.0, 0.0);
                monster.delay(0.2);
            }),
        ),
        (
            "hiparma:armagon_die8",
            Arc::new(|monster| {
                think(monster);
                monster.delay(2.0);
            }),
        ),
        ("hiparma:armagon_die14", Arc::new(final_death)),
    ];
    for left in [true, false] {
        let servo = servo.clone();
        actions.push((
            if left {
                "hiparma:armagon_overleft1"
            } else {
                "hiparma:armagon_overright1"
            },
            Arc::new(move |monster: &mut MissionMonster| {
                monster.entity.count = 0.0;
                monster.flush_entity();
                over_think(monster, left);
            }),
        ));
        actions.push((
            if left {
                "hiparma:armagon_overleft3"
            } else {
                "hiparma:armagon_overright3"
            },
            Arc::new(move |monster: &mut MissionMonster| {
                servo(monster);
                over_think(monster, left);
            }),
        ));
    }
    actions.extend([
        (
            "hiparma:armagon_overleft5",
            Arc::new({
                let foot = foot.clone();
                move |monster: &mut MissionMonster| {
                    foot(monster);
                    over_think(monster, true);
                }
            }) as MissionAction,
        ),
        (
            "hiparma:armagon_overleft11",
            Arc::new(|monster: &mut MissionMonster| {
                monster.entity.count = 1.0;
                monster.flush_entity();
                over_think(monster, true);
            }) as MissionAction,
        ),
        (
            "hiparma:armagon_overleft12",
            Arc::new(|monster: &mut MissionMonster| {
                monster.entity.count = 2.0;
                monster.flush_entity();
                over_think(monster, true);
            }) as MissionAction,
        ),
        (
            "hiparma:armagon_overright5",
            Arc::new({
                let foot = foot.clone();
                move |monster: &mut MissionMonster| {
                    foot(monster);
                    monster.entity.count = 1.0;
                    monster.flush_entity();
                    over_think(monster, false);
                }
            }) as MissionAction,
        ),
        (
            "hiparma:armagon_overright6",
            Arc::new(|monster: &mut MissionMonster| {
                monster.entity.count = 2.0;
                monster.flush_entity();
                over_think(monster, false);
            }) as MissionAction,
        ),
        (
            "hiparma:armagon_overright10",
            Arc::new({
                let foot = foot.clone();
                move |monster: &mut MissionMonster| {
                    foot(monster);
                    over_think(monster, false);
                }
            }) as MissionAction,
        ),
    ]);
    let spawn: MissionAction = Arc::new(move |monster| {
        let id = monster.entity.actor.id().clone();
        let origin = monster.origin;
        let Ok(body) = monster.game.create("armagon_body", None, None) else {
            return;
        };
        monster.lefty = false;
        let _ = monster.game.set_body(
            &body,
            &BodyPatch {
                origin: Some(vsub(
                    origin,
                    Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 64.0,
                    },
                )),
                bounds: Some(Bounds {
                    min: Vec3 {
                        x: -16.0,
                        y: -16.0,
                        z: -16.0,
                    },
                    max: Vec3 {
                        x: 16.0,
                        y: 16.0,
                        z: 16.0,
                    },
                }),
                ..Default::default()
            },
        );
        if monster.game.options().edition == Q1Edition::Rerelease {
            let _ = monster.game.update_entity(&body, |entity| {
                entity.fields.insert(
                    "oldorigin".to_string(),
                    format!("{} {} {}", origin.x, origin.y, origin.z),
                );
            });
        } else {
            let _ = monster.game.update_entity(&body, |entity| {
                entity.movement = Q1MoveType::Step;
            });
        }
        let _ = monster.game.update_entity(&body, |entity| {
            entity.solid = Q1Solid::None;
            entity.model = "progs/armabody.mdl".to_string();
            entity.references.insert("trigger_field".to_string(), Some(id.clone()));
        });
        monster
            .entity
            .references
            .insert("trigger_field".to_string(), Some(body.clone()));
        monster.flush_entity();
        number(monster, "fixangle", 0.0);
        number(
            monster,
            "yaw_speed",
            if skill == 0 {
                5.0
            } else if skill == 1 {
                9.0
            } else {
                12.0
            },
        );
        number(
            monster,
            "worldtype",
            if skill == 0 {
                0.9
            } else if skill == 1 {
                0.85
            } else {
                0.75
            },
        );
        number(monster, "state", 0.0);
        number(monster, "super_time", 0.0);
        number(monster, "endtime", 0.0);
        let _ = monster.game.link(&body);
        monster.spawn_default();
    });
    let found: MissionFound = Arc::new(|monster, target| {
        monster.found_default(target);
        sound(monster, "armagon/sight.wav", Q1SoundChannel::Voice, 1.0, 0.1);
    });
    let check_attack: MissionCheckAttack = Arc::new(check_attack);
    let melee: MissionAction = Arc::new(|monster| {
        monster.play("armagon_stop1");
    });
    let pain: MissionPain = Arc::new(|monster, _attacker, damage| {
        let id = monster.entity.actor.id().clone();
        if monster.game.health(&id) <= 0.0 || damage < 25.0 || monster.state.pain_finished > monster.game.time {
            return;
        }
        let time = monster.game.time;
        monster.state.pain_finished = time + 2.0;
        let _ = monster.game.sound_simple(&id, "armagon/pain.wav");
    });
    let die: MissionDie = Arc::new(|monster, _attacker| {
        monster.play("armagon_die1");
    });
    PackMonsterDefinition {
        spec: Box::leak(Box::new(MonsterSpecies {
            species: Q1MonsterSpecies::Armagon,
            kill_string: None,
            classnames: &["monster_armagon"],
            model: "armalegs",
            head: None,
            health: if skill == 0 {
                2000.0
            } else if skill == 1 {
                2500.0
            } else {
                3500.0
            },
            gib_health: f64::NEG_INFINITY,
            gibs: &[],
            bounds: Bounds {
                min: Vec3 {
                    x: -48.0,
                    y: -48.0,
                    z: -24.0,
                },
                max: Vec3 {
                    x: 48.0,
                    y: 48.0,
                    z: 84.0,
                },
            },
            stand: "armagon_stand1",
            walk: "armagon_walk1",
            run: "armagon_run1",
            sight: "",
            missile: Some("armagon_missile_attack"),
            melee: true,
            movement: MonsterMovement::Walk,
        })),
        base_behavior: false,
        frames: FRAMES,
        actions,
        callbacks: vec![
            (
                "armagon_body_explode1",
                Q1CallbackHandlers {
                    action: Some(body_explode),
                    ..Default::default()
                },
            ),
            (
                "armagon_body_explode2",
                Q1CallbackHandlers {
                    action: Some(body_explosion),
                    ..Default::default()
                },
            ),
        ],
        spawn: Some(spawn),
        start: None,
        pain,
        die,
        melee: Some(melee),
        check_attack: Some(check_attack),
        found: Some(found),
        ai: None,
        use_: None,
    }
}

#[cfg(test)]
mod tests {
    use super::armagon_definition;
    use crate::q1::missionpacks::monsters::runtime::Q1MissionPackMonsters;
    use crate::q1::missionpacks::monsters::types::MissionMonsterHooks;
    use crate::q1::missionpacks::types::{test_game, Q1MissionPack};

    #[test]
    fn armagon_registers_with_split_body() {
        let mut game = test_game();
        let mut runtime =
            Q1MissionPackMonsters::new(&mut game, Q1MissionPack::Hipnotic, MissionMonsterHooks::default())
                .expect("runtime");
        let definition = armagon_definition(&game, &runtime);
        assert_eq!(definition.spec.classnames, &["monster_armagon"]);
        assert_eq!(definition.spec.model, "armalegs");
        assert!(definition.spec.health >= 2000.0);
        assert!(!definition.frames.is_empty());
        assert_eq!(definition.actions.len(), 33 + 4 + 6);
        assert_eq!(definition.callbacks.len(), 2);
        runtime.register(&mut game, definition).expect("register");
        let id = game.create("monster_armagon", None, None).expect("create");
        let monster = runtime.require(&mut game, &id).expect("require");
        assert_eq!(monster.definition.spec.model, "armalegs");
    }
}
