//! Monster frame actions (`src/content/q1/base/monster-actions.ts`).
//!
//! Direct ports of monster frame actions. Copyright (C) 1996-2022 id
//! Software LLC. GPL-2.0-or-later.

use qa_core::identity::{same_actor, ActorId};
use qa_core::math::Vec3;

use crate::q1::base::monsters::BaseMonster;
use crate::q1::base::projectiles::{
    cast_lightning as cast_monster_lightning, create_missile, drop_backpack, launch_laser, launch_ogre_grenade,
    launch_spike, launch_vore_ball, launch_zombie_grenade, spawn_meat_spray, BackpackDrop, SpikeKind,
};
use crate::q1::foundation::entity::Q1MonsterSpecies;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::BodyPatch;
use crate::q1::foundation::types::{
    length, normalize, vadd, vscale, vsub, Q1Edition, Q1Effect, Q1MoveType, Q1Solid, Q1SoundChannel, ZERO,
};
use crate::q1::{q1_error, Q1Error};

use super::animation::MonsterAi;

fn owned(monster: &BaseMonster) -> Result<qa_core::identity::OwnedActor, Q1Error> {
    monster
        .game
        .entity_ref(&monster.id)
        .map(|entity| entity.actor.clone())
        .ok_or_else(|| q1_error("Missing Q1 entity"))
}

fn meat(monster: &mut BaseMonster, side: f64) -> Result<(), Q1Error> {
    let axes = monster.make_vectors()?;
    let lateral = if side == 1.0 {
        (monster.game.host.random() * 2.0 - 1.0) * 100.0
    } else {
        side
    };
    let origin = monster.origin()?;
    let to = vadd(origin, vscale(axes.forward, 16.0));
    spawn_meat_spray(monster.game, &monster.id.clone(), to, vscale(axes.right, lateral))?;
    Ok(())
}

fn chainsaw(monster: &mut BaseMonster, side: f64) -> Result<(), Q1Error> {
    let enemy = monster.monster.enemy.clone();
    let Some(enemy) = enemy else { return Ok(()) };
    if !monster.game.can_damage(&enemy, &monster.id.clone()) {
        return Ok(());
    }
    monster.ai(MonsterAi::Charge, 10.0)?;
    if monster.distance()? > 100.0 {
        return Ok(());
    }
    monster.melee(100.0, 4.0, 3, false)?;
    if side != 0.0 {
        meat(monster, side)?;
    }
    Ok(())
}

fn jump(monster: &mut BaseMonster, tar: bool) -> Result<(), Q1Error> {
    let body = monster.game.body(&monster.id.clone())?;
    if tar {
        monster
            .game
            .update_entity(&monster.id.clone(), |entity| entity.movement = Q1MoveType::Bounce)?;
    }
    monster.controller.counter = 0.0;
    monster
        .game
        .update_entity(&monster.id.clone(), |entity| entity.movement_flags &= !512)?;
    let axes = monster.make_vectors()?;
    let vz = if tar {
        200.0 + monster.game.host.random() as f32 * 150.0
    } else {
        250.0
    };
    let velocity = vadd(vscale(axes.forward, 600.0), Vec3 { x: 0.0, y: 0.0, z: vz });
    monster.game.set_body(
        &monster.id.clone(),
        &BodyPatch {
            origin: Some(vadd(body.origin, Vec3 { x: 0.0, y: 0.0, z: 1.0 })),
            velocity: Some(velocity),
            ground: Some(None),
            ..Default::default()
        },
    )?;
    let prefix = monster.prefix.clone();
    let touch = monster.game.named.touch(&format!("{prefix}:monster_jump_touch"))?;
    monster
        .game
        .update_entity(&monster.id.clone(), |entity| entity.touch = Some(touch))?;
    monster.game.link(&monster.id.clone())
}

/// Demon and tarbaby landing (`monsterJumpTouch`).
pub fn monster_jump_touch(monster: &mut BaseMonster, other: &ActorId) -> Result<(), Q1Error> {
    let tar = monster.spec.species == Q1MonsterSpecies::Tarbaby;
    if monster.game.health(&monster.id.clone()) <= 0.0 {
        return Ok(());
    }
    let damageable = monster
        .game
        .host
        .combat
        .read(other)
        .is_some_and(|state| state.can_take_damage);
    let classname = monster
        .game
        .entity_ref(&monster.id.clone())
        .map(|entity| entity.classname.clone())
        .unwrap_or_default();
    if damageable && (!tar || monster.game.host.classname(other) != classname) {
        let speed = f64::from(length(
            monster.game.body(&monster.id.clone()).map(|body| body.velocity)?,
        ));
        if speed > 400.0 {
            let damage = (if tar { 10.0 } else { 40.0 }) + 10.0 * monster.game.host.random();
            monster
                .game
                .damage_direct(other, Some(&monster.id.clone()), Some(&monster.id.clone()), damage);
            if tar {
                monster
                    .game
                    .sound(&monster.id.clone(), "blob/hit1.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
            }
        }
    } else if tar {
        monster
            .game
            .sound(&monster.id.clone(), "blob/land1.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
    }
    if monster.game.host.check_bottom(&monster.id.clone()) {
        monster.game.update_entity(&monster.id.clone(), |entity| {
            entity.touch = None;
            if !tar {
                entity.movement = Q1MoveType::Step;
            }
        })?;
        monster.controller.next_frame = (if tar { "tbaby_jump1" } else { "demon1_jump11" }).to_string();
        return monster.delay(0.1);
    }
    if monster
        .game
        .body(&monster.id.clone())
        .map(|body| body.ground)
        .ok()
        .flatten()
        .is_some()
    {
        monster.game.update_entity(&monster.id.clone(), |entity| {
            entity.touch = None;
            entity.movement = Q1MoveType::Step;
        })?;
        monster.controller.next_frame = (if tar { "tbaby_run1" } else { "demon1_jump1" }).to_string();
        return monster.delay(0.1);
    }
    Ok(())
}

fn wizard_fast(monster: &mut BaseMonster) -> Result<(), Q1Error> {
    let Some(enemy) = monster.monster.enemy.clone() else {
        return Ok(());
    };
    monster.game.sound(
        &monster.id.clone(),
        "wizard/wattack.wav",
        Q1SoundChannel::Weapon,
        1.0,
        1.0,
    )?;
    let axes = monster.make_vectors()?;
    for (side, delay) in [(1.0, 0.8), (-1.0, 0.3)] {
        let timer = monster.game.create("wizard_fastfire", None, None)?;
        monster
            .game
            .update_entity(&timer, |entity| entity.owner = Some(monster.id.clone()))?;
        let origin = monster.origin()?;
        let at = vadd(
            origin,
            vadd(
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 30.0,
                },
                vadd(vscale(axes.forward, 14.0), vscale(axes.right, 14.0 * side)),
            ),
        );
        monster.game.set_origin(&timer, at)?;
        super::creatures::set_wizard_shot(
            monster.game,
            &timer,
            super::creatures::WizardShot {
                enemy: enemy.clone(),
                right: vscale(axes.right, side),
            },
        )?;
        monster.game.schedule(&timer, delay, "base:wizard_fastfire")?;
    }
    Ok(())
}

fn hell_knight_shot(monster: &mut BaseMonster, offset: f64) -> Result<(), Q1Error> {
    let Some(target) = monster.target()? else { return Ok(()) };
    let origin = monster.origin()?;
    let delta = vsub(target, origin);
    let angle = Vec3 {
        x: (f64::from(delta.z).atan2(f64::from(delta.x).hypot(f64::from(delta.y))) * 180.0 / std::f64::consts::PI)
            as f32,
        y: (f64::from(delta.y).atan2(f64::from(delta.x)) * 180.0 / std::f64::consts::PI + offset * 6.0) as f32,
        z: 0.0,
    };
    let forward = monster.game.make_vectors(angle).forward;
    let body = monster.game.body(&monster.id.clone())?;
    let at = vadd(
        vadd(origin, vscale(vadd(body.bounds.min, body.bounds.max), 0.5)),
        vscale(forward, 20.0),
    );
    let direction = normalize(forward);
    let jittered = Vec3 {
        x: direction.x,
        y: direction.y,
        z: -direction.z + (monster.game.host.random() as f32 - 0.5) * 0.1,
    };
    launch_spike(
        monster.game,
        Some(&monster.id.clone()),
        at,
        vscale(jittered, 300.0),
        SpikeKind::Knight,
    )?;
    monster.game.sound(
        &monster.id.clone(),
        "hknight/attack1.wav",
        Q1SoundChannel::Weapon,
        1.0,
        1.0,
    )
}

/// Delayed wizard shot (`wizardFastFire`).
pub fn wizard_fast_fire(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let shot = super::creatures::wizard_shot(game, &id)?;
    let target = game.host.bodies.read(&shot.enemy);
    let origin = game.body(&id).map(|body| body.origin)?;
    let owner = game.entity_ref(&id).and_then(|entity| entity.owner.clone());
    if let Some(owner) = owner {
        if game.health(&owner) > 0.0 {
            if let Some(target) = target {
                if let Some(body) = game.host.bodies.read(&owner) {
                    game.effect(Q1Effect::Muzzleflash, body.origin, Some(&owner), 1);
                }
                let angles = if game.options().edition == Q1Edition::Rerelease {
                    Vec3 {
                        x: -target.angles.x,
                        y: target.angles.y,
                        z: target.angles.z,
                    }
                } else {
                    target.angles
                };
                game.make_vectors(angles);
                let direction = normalize(vsub(vsub(target.origin, vscale(shot.right, 13.0)), origin));
                game.sound(&id, "wizard/wattack.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
                launch_spike(game, Some(&owner), origin, vscale(direction, 600.0), SpikeKind::Wizard)?;
            }
        }
    }
    game.remove(&id)
}

fn boss_face(monster: &mut BaseMonster) -> Result<(), Q1Error> {
    let current = monster.monster.enemy.clone();
    let rescan = match current.clone() {
        None => true,
        Some(enemy) => monster.game.health(&enemy) <= 0.0 || monster.game.host.random() < 0.02,
    };
    if rescan {
        let players = (monster.game.host.players)();
        let start = current
            .as_ref()
            .and_then(|current| players.iter().position(|actor| same_actor(actor, current)))
            .map_or(-1, |index| index as i32);
        for step in 1..=players.len().min(4) {
            let candidate = &players[((start + step as i32) as usize) % players.len()];
            if monster.game.health(candidate) > 0.0 {
                monster.monster.enemy = Some(candidate.clone());
                break;
            }
        }
    }
    monster.face()
}

fn boss_missile(monster: &mut BaseMonster, offset: Vec3) -> Result<(), Q1Error> {
    let target = monster.target()?;
    let enemy = monster.monster.enemy.clone();
    let (Some(target), Some(enemy)) = (target, enemy) else {
        return Ok(());
    };
    let origin = monster.origin()?;
    let delta = vsub(target, origin);
    let angles = Vec3 {
        x: (f64::from(delta.z).atan2(f64::from(delta.x).hypot(f64::from(delta.y))) * 180.0 / std::f64::consts::PI)
            as f32,
        y: (f64::from(delta.y).atan2(f64::from(delta.x)) * 180.0 / std::f64::consts::PI) as f32,
        z: 0.0,
    };
    let axes = monster.game.make_vectors(angles);
    let at = vadd(
        origin,
        vadd(
            vscale(axes.forward, f64::from(offset.x)),
            vadd(
                vscale(axes.right, f64::from(offset.y)),
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: offset.z,
                },
            ),
        ),
    );
    let velocity = monster
        .game
        .host
        .bodies
        .read(&enemy)
        .map(|body| body.velocity)
        .unwrap_or(ZERO);
    let destination = if monster.game.options().skill > 1 {
        vadd(
            target,
            vscale(
                Vec3 {
                    x: velocity.x,
                    y: velocity.y,
                    z: 0.0,
                },
                f64::from(length(vsub(target, at))) / 300.0,
            ),
        )
    } else {
        target
    };
    let missile = create_missile(
        monster.game,
        Some(&monster.id),
        "chthon_lavaball",
        "lavaball",
        at,
        vscale(normalize(vsub(destination, at)), 300.0),
        6.0,
    )?;
    monster.game.update_entity(&missile, |entity| {
        entity.projectile = Some(crate::q1::foundation::entity::Q1ProjectileKind::Rocket);
        entity.angular_velocity = Vec3 {
            x: 200.0,
            y: 100.0,
            z: 300.0,
        };
    })?;
    monster
        .game
        .sound(&monster.id.clone(), "boss1/throw.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
    if monster.game.health(&enemy) <= 0.0 {
        return monster.play("boss_idle1");
    }
    Ok(())
}

fn knight_offset(name: &str) -> Option<f64> {
    match name {
        "hknight_magica7" | "hknight_magicb7" | "hknight_magicc6" => Some(-2.0),
        "hknight_magica8" | "hknight_magicb8" | "hknight_magicc7" => Some(-1.0),
        "hknight_magica9" | "hknight_magicb9" | "hknight_magicc8" => Some(0.0),
        "hknight_magica10" | "hknight_magicb10" | "hknight_magicc9" => Some(1.0),
        "hknight_magica11" | "hknight_magicb11" | "hknight_magicc10" => Some(2.0),
        "hknight_magica12" | "hknight_magicb12" | "hknight_magicc11" => Some(3.0),
        _ => None,
    }
}

/// Run a named frame action (`monsterAction`).
pub fn monster_action(monster: &mut BaseMonster, name: &str) -> Result<(), Q1Error> {
    if let Some(offset) = knight_offset(name) {
        return hell_knight_shot(monster, offset);
    }
    if name.starts_with("boss_idle") || name.starts_with("boss_missile") {
        if name == "boss_idle1" {
            if let Some(enemy) = monster.monster.enemy.clone() {
                if monster.game.health(&enemy) > 0.0 {
                    return monster.play("boss_missile1");
                }
            }
        }
        if name == "boss_missile9" {
            return boss_missile(
                monster,
                Vec3 {
                    x: 100.0,
                    y: 100.0,
                    z: 200.0,
                },
            );
        }
        if name == "boss_missile20" {
            return boss_missile(
                monster,
                Vec3 {
                    x: 100.0,
                    y: -100.0,
                    z: 200.0,
                },
            );
        }
        return boss_face(monster);
    }
    match name {
        "knight_runatk1" => {
            let sound = if monster.game.host.random() > 0.5 {
                "knight/sword2.wav"
            } else {
                "knight/sword1.wav"
            };
            monster
                .game
                .sound(&monster.id.clone(), sound, Q1SoundChannel::Weapon, 1.0, 1.0)?;
            monster.ai(MonsterAi::Charge, 20.0)
        }
        "enf_atk6" | "enf_atk10" => {
            let Some(target) = monster.target()? else { return Ok(()) };
            let axes = monster.make_vectors()?;
            let origin = monster.origin()?;
            monster
                .game
                .effect(Q1Effect::Muzzleflash, origin, Some(&monster.id.clone()), 1);
            monster.game.sound(
                &monster.id.clone(),
                "enforcer/enfire.wav",
                Q1SoundChannel::Weapon,
                1.0,
                1.0,
            )?;
            launch_laser(
                monster.game,
                Some(&monster.id.clone()),
                vadd(
                    origin,
                    vadd(
                        vscale(axes.forward, 30.0),
                        vadd(
                            vscale(axes.right, 8.5),
                            Vec3 {
                                x: 0.0,
                                y: 0.0,
                                z: 16.0,
                            },
                        ),
                    ),
                ),
                vsub(target, origin),
            )?;
            Ok(())
        }
        "enf_atk14" => {
            if monster.game.options().skill == 3 && !monster.monster.refired && monster.visible(None)? {
                monster.monster.refired = true;
                monster.controller.next_frame = String::from("enf_atk1");
            }
            Ok(())
        }
        "enf_die3" | "enf_fdie3" => {
            let origin = monster.origin()?;
            drop_backpack(
                monster.game,
                origin,
                &BackpackDrop {
                    cells: 5.0,
                    ..Default::default()
                },
                None,
            )?;
            Ok(())
        }
        "demon1_jump4" => jump(monster, false),
        "demon1_jump10" => monster.delay(3.0),
        "demon1_atta5" | "demon1_atta11" => {
            monster.face()?;
            let ideal = monster
                .game
                .entity_ref(&monster.id.clone())
                .map(|entity| entity.ideal_yaw)
                .unwrap_or(0.0);
            let owned = owned(monster)?;
            monster.game.host.walk_move(&owned, ideal, 12.0);
            if let Some(enemy) = monster.monster.enemy.clone() {
                if monster.distance()? <= 100.0 && monster.game.can_damage(&enemy, &monster.id.clone()) {
                    monster
                        .game
                        .sound(&monster.id.clone(), "demon/dhit2.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
                    let damage = 10.0 + 5.0 * monster.game.host.random();
                    monster
                        .game
                        .damage_direct(&enemy, Some(&monster.id.clone()), Some(&monster.id.clone()), damage);
                    meat(monster, if name == "demon1_atta5" { 200.0 } else { -200.0 })?;
                }
            }
            Ok(())
        }
        "ogre_swing5" | "ogre_swing6" | "ogre_swing7" | "ogre_swing8" | "ogre_swing9" | "ogre_swing10"
        | "ogre_swing11" => {
            chainsaw(
                monster,
                if name == "ogre_swing6" {
                    200.0
                } else if name == "ogre_swing10" {
                    -200.0
                } else {
                    0.0
                },
            )?;
            let angles = monster.game.body(&monster.id.clone()).map(|body| body.angles)?;
            let yaw = angles.y + monster.game.host.random() as f32 * 25.0;
            monster.game.set_body(
                &monster.id.clone(),
                &BodyPatch {
                    angles: Some(Vec3 {
                        x: angles.x,
                        y: yaw,
                        z: angles.z,
                    }),
                    ..Default::default()
                },
            )
        }
        "ogre_smash6" | "ogre_smash7" | "ogre_smash8" | "ogre_smash9" => chainsaw(monster, 0.0),
        "ogre_smash10" => chainsaw(monster, 1.0),
        "ogre_smash11" => {
            chainsaw(monster, 0.0)?;
            let roll = monster.game.host.random();
            monster.delay(0.1 + roll * 0.2)
        }
        "ogre_nail4" => launch_ogre_grenade(monster),
        "ogre_die3" | "ogre_bdie3" => {
            let origin = monster.origin()?;
            drop_backpack(
                monster.game,
                origin,
                &BackpackDrop {
                    rockets: 2.0,
                    ..Default::default()
                },
                None,
            )?;
            Ok(())
        }
        "hknight_walk1" => {
            if monster.game.host.random() < 0.2 {
                monster.game.sound_simple(&monster.id.clone(), "hknight/idle.wav")?;
            }
            monster.ai(MonsterAi::Walk, 2.0)
        }
        "hknight_run1" => {
            if monster.game.host.random() < 0.2 {
                monster.game.sound_simple(&monster.id.clone(), "hknight/idle.wav")?;
            }
            monster.ai(MonsterAi::Run, 20.0)?;
            if let Some(target) = monster.target()? {
                let origin = monster.origin()?;
                if monster.visible(None)?
                    && monster.game.time >= monster.monster.attack_finished
                    && (origin.z - target.z).abs() <= 20.0
                    && monster.distance()? >= 80.0
                {
                    monster.attack_finished(2.0);
                    return monster.play("hknight_char_a1");
                }
            }
            Ok(())
        }
        "hknight_char_b1" => {
            if monster.game.time > monster.monster.attack_finished {
                monster.attack_finished(3.0);
                monster.play("hknight_run1")?;
            } else {
                let sound = if monster.game.host.random() > 0.5 {
                    "knight/sword2.wav"
                } else {
                    "knight/sword1.wav"
                };
                monster
                    .game
                    .sound(&monster.id.clone(), sound, Q1SoundChannel::Weapon, 1.0, 1.0)?;
            }
            monster.ai(MonsterAi::Charge, 23.0)?;
            monster.ai(MonsterAi::Melee, 0.0)
        }
        "sham_smash10" => {
            monster.ai(MonsterAi::Charge, 0.0)?;
            if let Some(enemy) = monster.monster.enemy.clone() {
                if monster.distance()? <= 100.0 && monster.game.can_damage(&enemy, &monster.id.clone()) {
                    monster.melee(100.0, 40.0, 3, false)?;
                    monster.game.sound(
                        &monster.id.clone(),
                        "shambler/smack.wav",
                        Q1SoundChannel::Voice,
                        1.0,
                        1.0,
                    )?;
                    for _ in 0..2 {
                        let axes = monster.game.basis;
                        let origin = monster.origin()?;
                        let lateral = (monster.game.host.random() * 2.0 - 1.0) * 100.0;
                        spawn_meat_spray(
                            monster.game,
                            &monster.id.clone(),
                            vadd(origin, vscale(axes.forward, 16.0)),
                            vscale(axes.right, lateral),
                        )?;
                    }
                }
            }
            Ok(())
        }
        "sham_swingl7" | "sham_swingr7" => {
            monster.ai(MonsterAi::Charge, 10.0)?;
            if monster.monster.enemy.is_some() && monster.distance()? <= 100.0 {
                monster.melee(100.0, 20.0, 3, false)?;
                monster.game.sound(
                    &monster.id.clone(),
                    "shambler/smack.wav",
                    Q1SoundChannel::Voice,
                    1.0,
                    1.0,
                )?;
                meat(monster, if name == "sham_swingl7" { 250.0 } else { -250.0 })?;
            }
            Ok(())
        }
        "sham_swingl9" => {
            if monster.game.host.random() < 0.5 {
                monster.controller.next_frame = String::from("sham_swingr1");
            }
            Ok(())
        }
        "sham_swingr9" => {
            if monster.game.host.random() < 0.5 {
                monster.controller.next_frame = String::from("sham_swingl1");
            }
            Ok(())
        }
        "sham_magic3" => {
            monster.delay(0.3)?;
            let origin = monster.origin()?;
            monster
                .game
                .effect(Q1Effect::Muzzleflash, origin, Some(&monster.id.clone()), 1);
            monster.face()?;
            let light = monster.game.create("shambler_light", None, None)?;
            monster
                .game
                .update_entity(&monster.id.clone(), |entity| entity.owner = Some(light.clone()))?;
            monster
                .game
                .update_entity(&light, |entity| entity.model = String::from("progs/s_light.mdl"))?;
            let angles = monster.game.body(&monster.id.clone()).map(|body| body.angles)?;
            monster.game.set_body(
                &light,
                &BodyPatch {
                    origin: Some(origin),
                    angles: Some(angles),
                    ..Default::default()
                },
            )?;
            monster.game.link(&light)?;
            monster.game.schedule(&light, 0.7, "SUB_Remove")
        }
        "sham_magic4" | "sham_magic5" => {
            let origin = monster.origin()?;
            monster
                .game
                .effect(Q1Effect::Muzzleflash, origin, Some(&monster.id.clone()), 1);
            let owner = monster
                .game
                .entity_ref(&monster.id.clone())
                .and_then(|entity| entity.owner.clone());
            if let Some(light) = owner {
                if monster.game.entity_ref(&light).is_some() {
                    let frame = if name == "sham_magic4" { 1 } else { 2 };
                    monster.game.update_entity(&light, |entity| entity.frame = frame)?;
                }
            }
            Ok(())
        }
        "sham_magic6" => {
            let owner = monster
                .game
                .entity_ref(&monster.id.clone())
                .and_then(|entity| entity.owner.clone());
            if let Some(light) = owner {
                if monster.game.entity_ref(&light).is_some() {
                    monster.game.remove(&light)?;
                }
            }
            cast_monster_lightning(monster)?;
            monster.game.sound(
                &monster.id.clone(),
                "shambler/sboom.wav",
                Q1SoundChannel::Weapon,
                1.0,
                1.0,
            )
        }
        "sham_magic9" | "sham_magic10" => cast_monster_lightning(monster),
        "wiz_walk1" | "wiz_run1" | "wiz_side1" => {
            let rolled = monster.game.host.random() * 5.0;
            if monster.controller.idle_until < monster.game.time {
                monster.controller.idle_until = monster.game.time + 2.0;
                if rolled > 4.5 {
                    monster.game.sound(
                        &monster.id.clone(),
                        "wizard/widle1.wav",
                        Q1SoundChannel::Voice,
                        2.0,
                        1.0,
                    )?;
                }
                if rolled < 1.5 {
                    monster.game.sound(
                        &monster.id.clone(),
                        "wizard/widle2.wav",
                        Q1SoundChannel::Voice,
                        2.0,
                        1.0,
                    )?;
                }
            }
            Ok(())
        }
        "wiz_fast1" => wizard_fast(monster),
        "wiz_fast10" => {
            monster.attack_finished(2.0);
            monster.controller.sliding = monster.range_distance(None)? < 500.0 && monster.visible(None)?;
            monster.controller.next_frame = (if monster.controller.sliding {
                "wiz_side1"
            } else {
                "wiz_run1"
            })
            .to_string();
            Ok(())
        }
        "wiz_death1" => {
            let velocity = Vec3 {
                x: -200.0 + 400.0 * monster.game.host.random() as f32,
                y: -200.0 + 400.0 * monster.game.host.random() as f32,
                z: 100.0 + 100.0 * monster.game.host.random() as f32,
            };
            monster.game.set_body(
                &monster.id.clone(),
                &BodyPatch {
                    velocity: Some(velocity),
                    ground: Some(None),
                    ..Default::default()
                },
            )?;
            monster.game.sound_simple(&monster.id.clone(), "wizard/wdeath.wav")
        }
        "shal_attack9" => launch_vore_ball(monster),
        "tbaby_fly4" => {
            monster.controller.counter += 1.0;
            if monster.controller.counter == 4.0 {
                return monster.play("tbaby_jump5");
            }
            Ok(())
        }
        "tbaby_jump5" => jump(monster, true),
        "tbaby_die1" => monster.game.set_damageable(&monster.id.clone(), false),
        "tbaby_die2" => {
            monster
                .game
                .radius_damage(&monster.id.clone(), Some(&monster.id.clone()), 120.0, None, None, "");
            monster.game.sound_simple(&monster.id.clone(), "blob/death1.wav")?;
            let origin = monster.origin()?;
            let velocity = monster.game.body(&monster.id.clone()).map(|body| body.velocity)?;
            monster.game.effect(
                Q1Effect::TarExplosion,
                vsub(origin, vscale(normalize(velocity), 8.0)),
                None,
                1,
            );
            monster.game.remove(&monster.id.clone())
        }
        "f_attack3" | "f_attack9" | "f_attack15" => {
            if monster.monster.enemy.is_some() && monster.distance()? <= 60.0 {
                monster
                    .game
                    .sound(&monster.id.clone(), "fish/bite.wav", Q1SoundChannel::Voice, 1.0, 1.0)?;
                monster.melee(60.0, 3.0, 2, false)?;
            }
            Ok(())
        }
        "zombie_cruc2" | "zombie_cruc3" | "zombie_cruc4" | "zombie_cruc5" | "zombie_cruc6" => {
            let roll = monster.game.host.random();
            monster.delay(0.1 + roll * 0.1)
        }
        "zombie_run1" => {
            monster.controller.in_pain = 0.0;
            Ok(())
        }
        "zombie_atta13" => launch_zombie_grenade(
            monster,
            Vec3 {
                x: -10.0,
                y: -22.0,
                z: 30.0,
            },
        ),
        "zombie_attb14" => launch_zombie_grenade(
            monster,
            Vec3 {
                x: -10.0,
                y: -24.0,
                z: 29.0,
            },
        ),
        "zombie_attc12" => launch_zombie_grenade(
            monster,
            Vec3 {
                x: -12.0,
                y: -19.0,
                z: 29.0,
            },
        ),
        "zombie_paine1" => {
            let owned = owned(monster)?;
            monster.game.host.combat.set_health(&owned, 60.0)
        }
        "zombie_paine11" => {
            let owned = owned(monster)?;
            monster.game.host.combat.set_health(&owned, 60.0)?;
            monster.delay(5.1)
        }
        "zombie_paine12" => {
            let owned = owned(monster)?;
            monster.game.host.combat.set_health(&owned, 60.0)?;
            monster.game.sound(
                &monster.id.clone(),
                "zombie/z_idle.wav",
                Q1SoundChannel::Voice,
                2.0,
                1.0,
            )?;
            monster
                .game
                .update_entity(&monster.id.clone(), |entity| entity.solid = Q1Solid::Slidebox)?;
            if !monster.game.host.walk_move(&owned, 0.0, 0.0) {
                monster.controller.next_frame = String::from("zombie_paine11");
                monster
                    .game
                    .update_entity(&monster.id.clone(), |entity| entity.solid = Q1Solid::None)?;
            }
            monster.game.link(&monster.id.clone())
        }
        "boss_death9" => {
            let origin = monster.origin()?;
            monster.game.effect(Q1Effect::LavaSplash, origin, None, 1);
            Ok(())
        }
        "boss_death10" => {
            monster.count_kill()?;
            monster.game.remove(&monster.id.clone())
        }
        "old_thrash15" => {
            monster.controller.counter += 1.0;
            if monster.controller.counter != 3.0 {
                monster.controller.next_frame = String::from("old_thrash1");
            }
            Ok(())
        }
        "old_thrash20" => super::provider::finish_finale(monster),
        _ => Err(q1_error(format!("Q1 frame action is not implemented: {name}"))),
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::ProviderId;

    use super::*;
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::foundation::host::mock::mock_host;
    use crate::q1::foundation::types::{Q1Edition, Q1FoundationOptions, Q1PrecacheProgram};

    fn options() -> Q1FoundationOptions {
        Q1FoundationOptions {
            provider: None,
            precache_program: Some(Q1PrecacheProgram::Id1),
            edition: Q1Edition::Classic,
            physics_edition: None,
            skill: 1,
            deathmatch: 0,
            coop: false,
            campaign: ProviderId::new("q1", "campaign"),
            combat_provider: ProviderId::new("q1", "combat"),
            movement_provider: ProviderId::new("q1", "movement"),
            inventory_provider: ProviderId::new("q1", "inventory"),
            gravity: 800.0,
            max_clients: Some(4),
            no_exit: None,
            teamplay: None,
            aim_threshold: None,
        }
    }

    #[test]
    fn actions_cover_knight_offsets_and_deaths() {
        assert_eq!(knight_offset("hknight_magica7"), Some(-2.0));
        assert_eq!(knight_offset("hknight_magicc11"), Some(3.0));
        assert_eq!(knight_offset("knight_runatk1"), None);
        let (host, _) = mock_host();
        let mut game = Q1EntityServices::new(host, options()).expect("game");
        let _guard = Q1BaseGuard::register(&mut game, Q1BaseOptions::default()).expect("base");
        let knight = game.create("monster_knight", None, None).expect("knight");
        game.spawn_entity(&knight, None).expect("spawn");
        let mut monster = BaseMonster::load(&mut game, &knight).expect("load");
        assert!(monster_action(&mut monster, "bogus_action").is_err());
        monster_action(&mut monster, "zombie_run1").expect("pain reset");
        assert_eq!(monster.controller.in_pain, 0.0);
        monster.game.remove(&monster.id.clone()).expect("remove");
    }

    #[test]
    fn wizard_shot_round_trip_fires() {
        let (host, _) = mock_host();
        let mut game = Q1EntityServices::new(host, options()).expect("game");
        let _guard = Q1BaseGuard::register(&mut game, Q1BaseOptions::default()).expect("base");
        let wizard = game.create("monster_wizard", None, None).expect("wizard");
        game.spawn_entity(&wizard, None).expect("spawn");
        let player = game.create("player", None, None).expect("player");
        let mut monster = BaseMonster::load(&mut game, &wizard).expect("load");
        monster.monster.enemy = Some(player.clone());
        monster_action(&mut monster, "wiz_fast1").expect("fast");
        let timers: Vec<ActorId> = monster
            .game
            .entity_ids()
            .into_iter()
            .filter(|id| {
                monster
                    .game
                    .entity_ref(id)
                    .is_some_and(|entity| entity.classname == "wizard_fastfire")
            })
            .collect();
        assert_eq!(timers.len(), 2);
        let timer = timers[0].clone();
        monster.finish().expect("finish");
        wizard_fast_fire(&mut game, &timer).expect("fire");
        assert!(game.entity_ref(&timer).is_none());
    }
}
