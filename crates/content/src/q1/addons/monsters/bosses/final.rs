//! Q1 mg3 final boss (`src/content/q1/addons/monsters/bosses/final.ts`).
//!
//! `quakec_mg3/monsters/boss_final.qc`. GPL-2.0-or-later.

use std::collections::HashMap;
use std::sync::OnceLock;

use qa_core::identity::{same_actor, ActorId};
use qa_core::math::{Bounds, Vec3};

use crate::contract::{ArmorState, PoweredProtectionState, RegularArmorState};
use crate::q1::addons::campaign::{BLOODY_NIGHTMARE_ACTIVE, BLOODY_NIGHTMARE_DISCOVERED, BLOODY_NIGHTMARE_NEWGAME};
use crate::q1::addons::context::{
    addon_emit, fround, set_addon_number, set_addon_player_number, set_addon_vector, set_combat_team, Q1AddonEvent,
};
use crate::q1::addons::monsters::ai::{register_mg3_monster_source, Mg3ActionHandler, Mg3Monster, Mg3SourceHooks};
use crate::q1::base::creatures::{monster_controller, store_monster_controller};
use crate::q1::base::monsters::BaseMonsterState;
use crate::q1::base::projectiles::{launch_spike, spawn_meat_spray, SpikeKind};
use crate::q1::base::provider::update_base;
use crate::q1::base::species::{MonsterMovement, MonsterSpecies};
use crate::q1::foundation::callbacks::{Q1ActionHandler, Q1CallbackHandlers, Q1TouchHandler, Q1UseHandler};
use crate::q1::foundation::entity::{Q1MonsterSpecies, Q1ProjectileKind};
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::gameplay::BodyPatch;
use crate::q1::foundation::host::Q1Contents;
use crate::q1::foundation::spawns::spawn_teleport_fog;
use crate::q1::foundation::types::{
    length, normalize, vadd, vscale, vsub, Q1Effect, Q1Event, Q1MoveType, Q1Solid, Q1SoundChannel, Q1Weapon, POINT,
    ZERO,
};
use crate::q1::missionpacks::types::velocity_angles;
use crate::q1::Q1Error;

use super::effects::pain_lightning;
use super::frames::r#final::frames;
use super::oldnew::throw_gib_vector;
use super::registry::register_boss_controllers;
use super::sphere_points::sphere_point;

/// Final boss callback prefix.
const FINAL_PREFIX: &str = "mg3:final";

/// Final boss spawn defaults (`spec`).
const FINAL_SPEC: MonsterSpecies = MonsterSpecies {
    species: Q1MonsterSpecies::Boss,
    kill_string: None,
    classnames: &["monster_boss_final"],
    model: "boss",
    head: None,
    health: 12000.0,
    gib_health: f64::NEG_INFINITY,
    gibs: &[],
    bounds: Bounds {
        min: Vec3 {
            x: -128.0,
            y: -128.0,
            z: -24.0,
        },
        max: Vec3 {
            x: 128.0,
            y: 128.0,
            z: 256.0,
        },
    },
    stand: "boss_final_idle1",
    walk: "boss_final_idle1",
    run: "boss_final_missile1",
    sight: "",
    missile: Some("boss_final_missile1"),
    melee: false,
    movement: MonsterMovement::Boss,
};

/// Schedules a prefixed boss action (`later`).
fn later(game: &mut Q1EntityServices, id: &ActorId, name: &str, delay: f64) -> Result<(), Q1Error> {
    game.schedule(id, delay, &format!("{FINAL_PREFIX}:{name}"))
}

/// Signed unit random (`randomSigned`).
fn random_signed(game: &mut Q1EntityServices) -> f64 {
    fround(2.0 * game.host.random() - 1.0)
}

/// Spins a rock (`spin`).
fn spin_rock(game: &mut Q1EntityServices, shot: &ActorId) -> Result<(), Q1Error> {
    let x = random_signed(game);
    let y = random_signed(game);
    let z = random_signed(game);
    game.update_entity(shot, |entity| {
        entity.angular_velocity = Vec3 {
            x: (300.0 * x) as f32,
            y: (300.0 * y) as f32,
            z: (300.0 * z) as f32,
        };
    })
}

/// Fires a boss rock (`rock`).
fn fire_rock(
    game: &mut Q1EntityServices,
    owner: Option<&ActorId>,
    origin: Vec3,
    direction: Vec3,
    model: &str,
) -> Result<ActorId, Q1Error> {
    let shot = launch_spike(game, owner, origin, direction, SpikeKind::Spike)?;
    let touch = game.named.touch(&format!("{FINAL_PREFIX}:T_RockTouch2"))?;
    game.update_entity(&shot, |entity| {
        entity.classname = String::from("rock");
        entity.model = format!("progs/rogue/{model}.mdl");
        entity.touch = Some(touch);
    })?;
    game.set_bounds(&shot, POINT)?;
    Ok(shot)
}

/// Faces the boss at a living enemy (`bossFace`).
fn boss_face(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    let enemy = monster.monster.monster.enemy.clone();
    let health = enemy
        .as_ref()
        .map(|enemy| monster.monster.game.health(enemy))
        .unwrap_or(0.0);
    if health <= 0.0 || monster.monster.game.host.random() < 0.02 {
        let mut players = (monster.monster.game.host.players)();
        players.sort_by_key(|actor| actor.slot());
        let slot = enemy.as_ref().map(|enemy| enemy.slot()).unwrap_or(0);
        let next = players
            .iter()
            .find(|actor| actor.slot() > slot)
            .or_else(|| players.first())
            .cloned();
        monster.monster.monster.enemy = next;
    }
    let enemy = monster.monster.monster.enemy.clone();
    let health = enemy
        .as_ref()
        .map(|enemy| monster.monster.game.health(enemy))
        .unwrap_or(0.0);
    if health > 0.0 {
        monster.monster.face()?;
    }
    Ok(())
}

/// Resolves the boss muzzle (`muzzle`).
fn boss_muzzle(monster: &mut Mg3Monster, offset: Vec3, angles: Vec3) -> Result<Vec3, Q1Error> {
    let basis = monster.monster.game.make_vectors(angles);
    let origin = monster.monster.origin()?;
    Ok(vadd(
        vadd(
            vadd(origin, vscale(basis.forward, f64::from(offset.x))),
            vscale(basis.right, f64::from(offset.y)),
        ),
        Vec3 {
            x: 0.0,
            y: 0.0,
            z: offset.z,
        },
    ))
}

/// Fires a boss lava ball (`missile`).
fn boss_missile(monster: &mut Mg3Monster, offset: Vec3) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    let spawnflags = monster
        .monster
        .game
        .entity_ref(&id)
        .map(|entity| entity.spawnflags)
        .unwrap_or(0);
    if spawnflags & 2 != 0 {
        monster.monster.game.host.random();
        return boss_line(monster, offset);
    }
    let target = monster.monster.target()?.unwrap_or(ZERO);
    let origin = monster.monster.origin()?;
    let muzzle = boss_muzzle(monster, offset, velocity_angles(vsub(target, origin)))?;
    let direction = normalize(vsub(target, muzzle));
    let shot = launch_spike(monster.monster.game, Some(&id), muzzle, direction, SpikeKind::Spike)?;
    let touch = monster.monster.game.named.touch("projectile_touch")?;
    monster.monster.game.update_entity(&shot, |entity| {
        entity.model = String::from("progs/lavaball.mdl");
        entity.projectile = Some(Q1ProjectileKind::Rocket);
        entity.touch = Some(touch);
        entity.angular_velocity = Vec3 {
            x: 200.0,
            y: 100.0,
            z: 300.0,
        };
    })?;
    monster.monster.game.set_body(
        &shot,
        &BodyPatch {
            bounds: Some(POINT),
            velocity: Some(vscale(direction, 300.0)),
            ..Default::default()
        },
    )?;
    monster
        .monster
        .game
        .sound(&id, "boss1/throw.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
    let enemy = monster.monster.monster.enemy.clone();
    let health = enemy
        .as_ref()
        .map(|enemy| monster.monster.game.health(enemy))
        .unwrap_or(0.0);
    if health <= 0.0 {
        monster.play("boss_final_idle1")?;
    }
    Ok(())
}

/// Fires the boss line barrage (`line`).
fn boss_line(monster: &mut Mg3Monster, offset: Vec3) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    let skill = monster.monster.game.options().skill;
    let count = if skill == 0 {
        4
    } else if skill == 1 {
        8
    } else if skill == 3 {
        15
    } else {
        11
    };
    let angles = monster.monster.game.body(&id)?.angles;
    let origin = boss_muzzle(monster, offset, angles)?;
    let basis = monster.monster.game.make_vectors(angles);
    let target = monster.monster.target()?.unwrap_or(ZERO);
    let flat = vsub(target, origin);
    let direction = normalize(Vec3 {
        x: flat.x,
        y: flat.y,
        z: 0.0,
    });
    monster.monster.game.effect(Q1Effect::Explosion, origin, None, 1);
    let sign = if offset.y < 0.0 { 1.0 } else { -1.0 };
    let start = 4.0 / f64::from(count) * sign;
    let increment = 2.0 / f64::from(count) * -sign;
    for i in 0..count {
        let damage = 10.0 * monster.monster.game.host.random() + 10.0;
        boss_radius_damage(monster.monster.game, &id, damage);
        let aim = normalize(vadd(
            vadd(direction, vscale(basis.right, start)),
            vscale(vscale(basis.right, increment), f64::from(i)),
        ));
        let aim = normalize(Vec3 {
            x: aim.x,
            y: aim.y,
            z: 0.0,
        });
        let shot = fire_rock(
            monster.monster.game,
            Some(&id),
            vadd(origin, vscale(aim, 8.0)),
            aim,
            "sphere",
        )?;
        let signed = random_signed(monster.monster.game);
        let mut velocity = vscale(aim, 300.0 + signed * 100.0);
        if monster.monster.game.host.random() > 0.5 {
            velocity.z = ((monster.monster.game.host.random() + 1.0) * 8.0) as f32;
        }
        monster.monster.game.set_body(
            &shot,
            &BodyPatch {
                velocity: Some(velocity),
                ..Default::default()
            },
        )?;
        spin_rock(monster.monster.game, &shot)?;
        if i % 2 == 0 {
            set_addon_number(monster.monster.game, &shot, "frags", 1.0)?;
            monster
                .monster
                .game
                .update_entity(&shot, |entity| entity.effects |= 64)?;
        }
    }
    Ok(())
}

/// Fires the boss scatter blast (`blast`).
fn boss_blast(monster: &mut Mg3Monster, offset: Vec3, spread: f64, effect: bool) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    let skill = monster.monster.game.options().skill;
    let speed = if skill > 2 {
        500.0
    } else if skill > 0 {
        450.0
    } else {
        400.0
    };
    let width = fround(0.15 + fround(0.2 * spread.abs()));
    let count = (monster.monster.game.host.random() * 6.0 + 0.5).floor() as i64;
    let target = monster.monster.target()?.unwrap_or(ZERO);
    let origin = monster.monster.origin()?;
    let angles = velocity_angles(vsub(target, origin));
    let muzzle = boss_muzzle(monster, offset, angles)?;
    let basis = monster.monster.game.make_vectors(angles);
    let time = fround(f64::from(length(vsub(target, muzzle))) / speed);
    let enemy = monster.monster.monster.enemy.clone();
    let velocity = enemy
        .as_ref()
        .and_then(|enemy| monster.monster.game.host.bodies.read(enemy))
        .map(|body| body.velocity)
        .unwrap_or(ZERO);
    let flat = Vec3 {
        x: velocity.x,
        y: velocity.y,
        z: 0.0,
    };
    let direction = normalize(vsub(vadd(target, vscale(flat, time / 4.0)), muzzle));
    if effect {
        monster.monster.game.effect(Q1Effect::Explosion, muzzle, None, 1);
    }
    for remaining in (1..=count).rev() {
        let jitter_right = random_signed(monster.monster.game);
        let jitter_up = random_signed(monster.monster.game);
        let aim = normalize(vadd(
            vadd(direction, vscale(basis.right, jitter_right * width)),
            vscale(basis.up, jitter_up * width),
        ));
        let shot = fire_rock(
            monster.monster.game,
            Some(&id),
            vadd(muzzle, vscale(aim, 8.0)),
            aim,
            "sphere",
        )?;
        let jitter_speed = random_signed(monster.monster.game);
        monster.monster.game.set_body(
            &shot,
            &BodyPatch {
                velocity: Some(vscale(aim, speed + jitter_speed * 100.0)),
                ..Default::default()
            },
        )?;
        spin_rock(monster.monster.game, &shot)?;
        if remaining % 2 == 0 {
            monster
                .monster
                .game
                .update_entity(&shot, |entity| entity.effects |= 64)?;
        }
    }
    Ok(())
}

/// Upgrades the boss shock stage (`upgrade`).
fn boss_upgrade(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    monster.monster.game.update_entity(&id, |entity| {
        entity.wait += 1.0;
    })?;
    set_addon_number(monster.monster.game, &id, "boss_immune", 0.0)?;
    let wait = monster
        .monster
        .game
        .entity_ref(&id)
        .map(|entity| entity.wait)
        .unwrap_or(0.0);
    let target = monster.monster.game.entity_ref(&id).and_then(|entity| {
        if wait == 2.0 {
            Some(entity.text("wave1"))
        } else if wait == 3.0 {
            Some(entity.text("tele_target"))
        } else if wait == 4.0 {
            Some(entity.text("wave2"))
        } else if wait == 5.0 {
            Some(entity.text("wave3"))
        } else {
            None
        }
    });
    if wait == 3.0 {
        monster.monster.game.update_entity(&id, |entity| entity.frame = 0)?;
        monster.monster.game.schedule(&id, 0.0, "SUB_Null")?;
        set_addon_number(monster.monster.game, &id, "boss_immune", 1.0)?;
    }
    if let Some(target) = target {
        if !target.is_empty() {
            let original = monster
                .monster
                .game
                .entity_ref(&id)
                .map(|entity| entity.target.clone())
                .unwrap_or_default();
            monster
                .monster
                .game
                .update_entity(&id, |entity| entity.target = target)?;
            let activator = monster
                .monster
                .game
                .entity_ref(&id)
                .and_then(|entity| entity.activator.clone());
            monster.monster.game.use_targets(&id, activator.as_ref())?;
            monster
                .monster
                .game
                .update_entity(&id, |entity| entity.target = original)?;
        }
    }
    if wait == 3.0 {
        boss_teleport(monster.monster.game, false)?;
    }
    if wait == 5.0 {
        let origin = monster.monster.origin()?;
        let target = monster.monster.target()?.unwrap_or(ZERO);
        let flat = vsub(target, origin);
        let spiral = monster.monster.game.create("spiral", None, None)?;
        monster.monster.game.update_entity(&spiral, |entity| {
            entity.classname = String::new();
            entity.owner = Some(id.clone());
        })?;
        monster.monster.game.set_body(
            &spiral,
            &BodyPatch {
                origin: Some(vadd(
                    origin,
                    Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 60.0,
                    },
                )),
                angles: Some(velocity_angles(normalize(Vec3 {
                    x: flat.x,
                    y: flat.y,
                    z: 0.0,
                }))),
                ..Default::default()
            },
        )?;
        let delay = if monster.monster.game.options().skill <= 2 {
            0.2
        } else {
            0.15
        };
        monster.monster.game.update_entity(&spiral, |entity| {
            entity.delay = delay;
            entity.count = 50.0;
        })?;
        later(monster.monster.game, &spiral, "spiral_single", delay)?;
    }
    Ok(())
}

/// Final boss frame actions (`actions`).
fn final_actions() -> &'static HashMap<String, Mg3ActionHandler> {
    static ACTIONS: OnceLock<HashMap<String, Mg3ActionHandler>> = OnceLock::new();
    ACTIONS.get_or_init(|| {
        fn rise1(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            monster
                .monster
                .game
                .sound(&id, "boss1/out1.wav", Q1SoundChannel::Weapon, 1.0, 1.0)
        }
        fn rise2(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            monster
                .monster
                .game
                .sound(&id, "boss1/sight1.wav", Q1SoundChannel::Voice, 1.0, 1.0)
        }
        fn mg7(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            let skill = monster.monster.game.options().skill;
            let shells = fround(fround(f64::from(skill + 3) / 6.0) * 30.0)
                + (random_signed(monster.monster.game) * 10.0 + 0.5).floor();
            set_addon_number(monster.monster.game, &id, "ammo_shells", shells)?;
            set_addon_number(monster.monster.game, &id, "ammo_nails", shells)?;
            if skill > 0 {
                monster.attack_finished(5.0);
            }
            boss_face(monster)
        }
        fn mg8(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            let (nails, shells) = monster
                .monster
                .game
                .entity_ref(&id)
                .map(|entity| (entity.number("ammo_nails"), entity.number("ammo_shells")))
                .unwrap_or((0.0, 0.0));
            let wide = nails % 2.0 == 0.0;
            let mut spread = fround(1.0 - fround(nails / shells));
            spread = fround(spread * spread);
            boss_blast(
                monster,
                Vec3 {
                    x: 270.0,
                    y: 60.0,
                    z: 210.0,
                },
                if wide { spread } else { spread * 0.5 },
                wide,
            )?;
            if nails != 0.0 {
                monster.monster.controller.next_frame = String::from("boss_final_mg8");
            }
            set_addon_number(monster.monster.game, &id, "ammo_nails", nails - 1.0)?;
            boss_face(monster)
        }
        fn missile1(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            set_addon_number(monster.monster.game, &id, "boss_immune", 0.0)?;
            boss_face(monster)
        }
        fn missile10(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            boss_missile(
                monster,
                Vec3 {
                    x: 200.0,
                    y: 100.0,
                    z: 60.0,
                },
            )
        }
        fn missile21(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            boss_missile(
                monster,
                Vec3 {
                    x: 200.0,
                    y: -100.0,
                    z: 60.0,
                },
            )
        }
        fn death1(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            monster
                .monster
                .game
                .sound(&id, "boss1/death.wav", Q1SoundChannel::Voice, 1.0, 1.0)?;
            let origin = monster.monster.origin()?;
            for delay in [1.0, 3.0, 5.0] {
                let timer = monster.monster.game.create("circle_thinker", None, None)?;
                monster
                    .monster
                    .game
                    .update_entity(&timer, |entity| entity.classname = String::new())?;
                monster.monster.game.set_origin(
                    &timer,
                    vadd(
                        origin,
                        Vec3 {
                            x: 0.0,
                            y: 0.0,
                            z: 60.0,
                        },
                    ),
                )?;
                later(monster.monster.game, &timer, "circle_think", delay)?;
            }
            Ok(())
        }
        fn death9(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            monster
                .monster
                .game
                .sound(&id, "boss1/out1.wav", Q1SoundChannel::Body, 1.0, 1.0)?;
            let origin = monster.monster.origin()?;
            monster.monster.game.effect(Q1Effect::LavaSplash, origin, None, 1);
            Ok(())
        }
        fn death10(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            monster.monster.game.killed_monsters += 1;
            let (total, found) = (
                monster.monster.game.total_monsters,
                monster.monster.game.killed_monsters,
            );
            monster.monster.game.host.emit(Q1Event::MonsterKilled {
                actor: id.clone(),
                total,
                found,
            });
            let frags = monster
                .monster
                .game
                .entity_ref(&id)
                .map(|entity| entity.number("frags"))
                .unwrap_or(0.0);
            set_addon_number(monster.monster.game, &id, "frags", frags + 1.0)?;
            loop {
                let frags = monster
                    .monster
                    .game
                    .entity_ref(&id)
                    .map(|entity| entity.number("frags"))
                    .unwrap_or(0.0);
                if frags >= 5.0 {
                    break;
                }
                let activator = monster
                    .monster
                    .game
                    .entity_ref(&id)
                    .and_then(|entity| entity.activator.clone());
                monster.monster.game.use_targets(&id, activator.as_ref())?;
                set_addon_number(monster.monster.game, &id, "frags", frags + 1.0)?;
            }
            monster
                .monster
                .game
                .sound(&id, "boss2/pop2.wav", Q1SoundChannel::Voice, 1.0, 1.0)?;
            let origin = monster.monster.origin()?;
            for index in 0..100 {
                let point = sphere_point(index)?;
                if point.z > 0.0 {
                    throw_gib_vector(
                        monster.monster.game,
                        vadd(
                            vadd(origin, vscale(point, 64.0)),
                            Vec3 {
                                x: 0.0,
                                y: 0.0,
                                z: 48.0,
                            },
                        ),
                        point,
                    )?;
                }
            }
            let timer = monster.monster.game.create("boss_end_timer", None, None)?;
            monster
                .monster
                .game
                .update_entity(&timer, |entity| entity.classname = String::new())?;
            monster.monster.game.schedule(&timer, 8.0, "mg3:bosses:boss_end")?;
            monster.monster.game.remove(&id)
        }
        HashMap::from([
            (String::from("boss_final_rise1"), rise1 as Mg3ActionHandler),
            (String::from("boss_final_rise2"), rise2 as Mg3ActionHandler),
            (String::from("boss_final_mg7"), mg7 as Mg3ActionHandler),
            (String::from("boss_final_mg8"), mg8 as Mg3ActionHandler),
            (String::from("boss_final_missile1"), missile1 as Mg3ActionHandler),
            (String::from("boss_final_missile10"), missile10 as Mg3ActionHandler),
            (String::from("boss_final_missile21"), missile21 as Mg3ActionHandler),
            (String::from("boss_final_death1"), death1 as Mg3ActionHandler),
            (String::from("boss_final_death9"), death9 as Mg3ActionHandler),
            (String::from("boss_final_death10"), death10 as Mg3ActionHandler),
            (String::from("boss_final_decide"), decide as Mg3ActionHandler),
        ])
    })
}

/// Final boss decision (`boss_final_decide` action).
fn decide(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    monster.play("boss_final_decide")
}

fn final_load_controller(
    game: &Q1EntityServices,
    id: &ActorId,
    _classname: &str,
) -> Result<(BaseMonsterState, &'static MonsterSpecies), Q1Error> {
    let controller = monster_controller(game, id, "monster_boss_final")?;
    Ok((controller, &FINAL_SPEC))
}

fn final_play(monster: &mut Mg3Monster, name: &str) -> Result<(), Q1Error> {
    if name == "boss_final_decide" {
        let id = monster.monster.id.clone();
        let wait = monster
            .monster
            .game
            .entity_ref(&id)
            .map(|entity| entity.wait)
            .unwrap_or(0.0);
        let pick_mg = wait > 0.0 && monster.monster.game.host.random() < 0.3;
        return monster.play_default(if pick_mg {
            "boss_final_mg1"
        } else {
            "boss_final_missile1"
        });
    }
    if name.starts_with("boss_final_idle") {
        boss_face(monster)?;
        if name == "boss_final_idle31" {
            let enemy = monster.monster.monster.enemy.clone();
            return monster.play(if enemy.is_some() {
                "boss_final_missile1"
            } else {
                "boss_final_idle1"
            });
        }
        return Ok(());
    }
    if name.starts_with("boss_final_shock") {
        if name.ends_with("10") {
            return boss_upgrade(monster);
        }
        let id = monster.monster.id.clone();
        let context = crate::q1::addons::context::Q1AddonContext::new(crate::q1::addons::context::Q1AddonProgram::Mg3);
        return pain_lightning(
            &context,
            monster.monster.game,
            &id,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 100.0,
            },
        );
    }
    if name.starts_with("boss_final_missile") || name.starts_with("boss_final_mg") {
        return boss_face(monster);
    }
    monster.play_default(name)
}

fn final_awake(monster: &mut Mg3Monster, activator: Option<&ActorId>) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    monster.monster.game.update_entity(&id, |entity| {
        entity.solid = Q1Solid::Slidebox;
        entity.movement = Q1MoveType::Step;
        entity.model = String::from("progs/boss.mdl");
    })?;
    monster.monster.game.set_damageable(&id, true)?;
    monster.monster.game.update_entity(&id, |entity| {
        entity.aimed_damage = true;
    })?;
    monster.attack_finished(3.0);
    monster.monster.game.set_bounds(&id, FINAL_SPEC.bounds)?;
    monster.monster.monster.enemy = activator.cloned();
    let die = monster.monster.game.named.die(&format!("{FINAL_PREFIX}:monster_die"))?;
    let pain = monster
        .monster
        .game
        .named
        .pain(&format!("{FINAL_PREFIX}:monster_pain"))?;
    let use_callback = monster
        .monster
        .game
        .named
        .use_callback(&format!("{FINAL_PREFIX}:monster_use"))?;
    monster.monster.game.update_entity(&id, |entity| {
        entity.die = Some(die);
        entity.pain = Some(pain);
        entity.use_callback = Some(use_callback);
        entity.yaw_speed = 20.0;
    })?;
    let owned = monster.monster.game.entity_ref(&id).map(|entity| entity.actor.clone());
    if let Some(owned) = owned {
        monster.monster.game.host.combat.set_health(&owned, 12000.0)?;
    }
    monster
        .monster
        .game
        .update_entity(&id, |entity| entity.max_health = 12000.0)?;
    let origin = monster.monster.origin()?;
    monster.monster.game.effect(Q1Effect::LavaSplash, origin, None, 1);
    monster.play("boss_final_rise1")
}

fn final_awake_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let activator = activator.cloned();
    let mut monster = Mg3Monster::load(game, id)?;
    final_awake(&mut monster, activator.as_ref())?;
    monster.finish()
}

fn final_pain(monster: &mut Mg3Monster, _attacker: Option<&ActorId>, _damage: f64) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    let immune = monster
        .monster
        .game
        .entity_ref(&id)
        .map(|entity| entity.number("boss_immune"))
        .unwrap_or(0.0);
    if monster.monster.monster.attack_finished > monster.monster.game.time
        || monster.monster.monster.pain_finished > monster.monster.game.time
        || immune != 0.0
    {
        return Ok(());
    }
    let mut event = String::new();
    let mut frags = monster
        .monster
        .game
        .entity_ref(&id)
        .map(|entity| entity.number("frags"))
        .unwrap_or(0.0);
    if frags == 0.0 {
        event = String::from("boss_final_shocka1");
        frags = 1.0;
    }
    let health = monster.monster.game.health(&id);
    let max_health = monster
        .monster
        .game
        .entity_ref(&id)
        .map(|entity| entity.max_health)
        .unwrap_or(0.0);
    for (step, fraction, frame) in [
        (1.0, 0.83, "boss_final_shocka1"),
        (2.0, 0.66, "boss_final_shocka1"),
        (3.0, 0.5, "boss_final_shockb1"),
        (4.0, 0.33, "boss_final_shocka1"),
        (5.0, 0.16, "boss_final_shockb1"),
    ] {
        if frags == step && health < fround(max_health * fround(fraction)) {
            event = String::from(frame);
            frags += 1.0;
        }
    }
    set_addon_number(monster.monster.game, &id, "frags", frags)?;
    if event.is_empty() {
        return Ok(());
    }
    set_addon_number(monster.monster.game, &id, "boss_immune", 1.0)?;
    monster
        .monster
        .game
        .sound(&id, "boss1/pain.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
    let time = monster.monster.game.time;
    monster.monster.monster.pain_finished = time + 3.0;
    monster.play(&event)
}

fn final_die(monster: &mut Mg3Monster, attacker: Option<&ActorId>) -> Result<(), Q1Error> {
    monster.monster.monster.enemy = attacker.cloned();
    monster
        .monster
        .game
        .set_damageable(&monster.monster.id.clone(), false)?;
    let id = monster.monster.id.clone();
    monster.monster.game.update_entity(&id, |entity| {
        entity.touch = None;
        entity.movement_flags &= !3;
    })?;
    let enemy = monster.monster.monster.enemy.clone();
    monster.monster.game.use_targets(&id, enemy.as_ref())?;
    monster.play("boss_final_death1")
}

/// Final boss melee check (`meleeCheck`).
fn final_melee_check(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    let enemy = monster.monster.monster.enemy.clone();
    let distance = monster.monster.distance()?;
    if distance > 300.0 || enemy.is_none() {
        return Ok(());
    }
    let enemy = enemy.unwrap_or_else(|| id.clone());
    if !monster.monster.game.can_damage(&enemy, &id) {
        return Ok(());
    }
    let damage =
        (monster.monster.game.host.random() + monster.monster.game.host.random() + monster.monster.game.host.random())
            * 40.0;
    monster
        .monster
        .game
        .damage(&enemy, Some(&id), Some(&id), damage, &Q1DamageParams::default());
    monster
        .monster
        .game
        .sound(&id, "shambler/smack.wav", Q1SoundChannel::Voice, 1.0, 1.0)?;
    let origin = monster.monster.origin()?;
    for _ in 0..2 {
        let forward = monster.monster.game.basis.forward;
        let right = monster.monster.game.basis.right;
        let jitter = random_signed(monster.monster.game);
        spawn_meat_spray(
            monster.monster.game,
            &id,
            vadd(origin, vscale(forward, 100.0)),
            vscale(right, jitter * 100.0),
        )?;
    }
    Ok(())
}

fn final_melee_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let mut monster = Mg3Monster::load(game, id)?;
    final_melee_check(&mut monster)?;
    monster.finish()
}

fn final_rise_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let mut monster = Mg3Monster::load(game, id)?;
    monster.play("boss_final_rise1")?;
    monster.finish()
}

fn spawn_final(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let monster = Mg3Monster::spawn_new(game, FINAL_PREFIX, id, &FINAL_SPEC)?;
    if monster.monster.game.options().deathmatch != 0 {
        let id = monster.monster.id.clone();
        monster.monster.game.remove(&id)?;
        return monster.finish();
    }
    let id = monster.monster.id.clone();
    monster.monster.game.update_entity(&id, |entity| {
        entity.classname = String::from("monster_boss");
        entity.movement_flags = 32;
    })?;
    set_addon_number(monster.monster.game, &id, "boss_immune", 1.0)?;
    let owned = monster.monster.game.entity_ref(&id).map(|entity| entity.actor.clone());
    if let Some(owned) = owned {
        set_combat_team(monster.monster.game, &owned, Some("q1:monsters"))?;
    }
    monster.monster.game.total_monsters += 1;
    let total = monster.monster.game.total_monsters;
    addon_emit(monster.monster.game, Q1AddonEvent::MonsterCount { count: total })?;
    set_addon_number(monster.monster.game, &id, "frags", 0.0)?;
    let awake = monster
        .monster
        .game
        .named
        .use_callback(&format!("{FINAL_PREFIX}:final_awake"))?;
    monster
        .monster
        .game
        .update_entity(&id, |entity| entity.use_callback = Some(awake))?;
    monster.finish()
}

/// Boss radius damage (`bossRadiusDamage`).
fn boss_radius_damage(game: &mut Q1EntityServices, source: &ActorId, damage: f64) {
    let origin = match game.body(source).map(|body| body.origin) {
        Ok(origin) => origin,
        Err(_) => return,
    };
    let source_id = source.clone();
    let mut observations = game.host.actors.observations();
    observations.reverse();
    for observation in observations {
        let target = observation.id;
        if same_actor(&target, &source_id)
            || game.host.classname(&target) == "monster_lava_man"
            || !game
                .host
                .combat
                .read(&target)
                .is_some_and(|combat| combat.can_take_damage)
        {
            continue;
        }
        let body = match game.host.bodies.read(&target) {
            Some(body) => body,
            None => continue,
        };
        let middle = vadd(body.origin, vscale(vadd(body.bounds.min, body.bounds.max), 0.5));
        if f64::from(length(vsub(origin, middle))) > damage + 40.0 {
            continue;
        }
        let points = damage - 0.5 * f64::from(length(vsub(origin, middle)));
        if points > 0.0 && game.can_damage(&target, &source_id) {
            game.damage(
                &target,
                Some(&source_id),
                Some(&source_id),
                points,
                &Q1DamageParams::default(),
            );
        }
    }
}

/// Teleports the boss arena (`bossTeleport`).
pub fn boss_teleport(game: &mut Q1EntityServices, variant: bool) -> Result<(), Q1Error> {
    if variant {
        let ids = game.entity_ids();
        let boss = ids.iter().find(|id| {
            game.entity_ref(id)
                .is_some_and(|entity| entity.classname == "monster_boss")
        });
        let point = ids.iter().find(|id| {
            game.entity_ref(id)
                .is_some_and(|entity| entity.classname == "info_boss_teleport_boss")
        });
        if let (Some(boss), Some(point)) = (boss, point) {
            let origin = game.body(point)?.origin;
            game.set_origin(boss, origin)?;
            let origin = game.body(boss)?.origin;
            game.effect(Q1Effect::LavaSplash, origin, None, 1);
            later(game, boss, "boss_final_rise1", 0.1)?;
        }
    }
    let ids = game.entity_ids();
    let classname = if variant {
        "info_boss_teleport_second"
    } else {
        "info_boss_teleport_first"
    };
    let points: Vec<ActorId> = ids
        .into_iter()
        .filter(|id| game.entity_ref(id).is_some_and(|entity| entity.classname == classname))
        .collect();
    for point in &points {
        game.update_entity(point, |entity| entity.wait = 0.0)?;
    }
    let players = (game.host.players)();
    for player in players {
        let slot = points
            .iter()
            .find(|point| game.entity_ref(point).is_some_and(|entity| entity.wait == 0.0));
        let Some(slot) = slot else {
            addon_emit(
                game,
                Q1AddonEvent::DeveloperMessage {
                    text: String::from("ERROR: Could not find valid teleport destination!\n"),
                },
            )?;
            break;
        };
        game.update_entity(slot, |entity| entity.wait = 1.0)?;
        let origin = game.body(slot)?.origin;
        let angles = game
            .entity_ref(slot)
            .map(|entity| entity.vector("mangle"))
            .unwrap_or(ZERO);
        game.update_entity(&player, |entity| entity.movement_flags &= !1)?;
        game.update_player(&player, |state| state.view_angles = angles)?;
        let owned = game.host.actors.resolve_owned(&player);
        let body = game.host.bodies.read(&player);
        if let (Some(owned), Some(body)) = (owned, body) {
            game.host.bodies.write(
                &owned,
                &crate::q1::foundation::gameplay::BodyState {
                    origin,
                    angles,
                    velocity: Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 300.0,
                    },
                    ground: None,
                    ..body
                },
            )?;
            game.host.bodies.link(&owned)?;
        }
        game.host.emit(Q1Event::Camera {
            player: player.clone(),
            origin,
            angles,
            view_offset: None,
        });
        let forward = game.make_vectors(angles).forward;
        spawn_teleport_fog(game, vadd(origin, vscale(forward, 32.0)))?;
    }
    Ok(())
}

fn spawn_boss_teleport_point(game: &mut Q1EntityServices, id: &ActorId, lift: f32) -> Result<(), Q1Error> {
    let angles = game.body(id)?.angles;
    set_addon_vector(game, id, "mangle", angles)?;
    game.update_entity(id, |entity| entity.model = String::new())?;
    let origin = game.body(id)?.origin;
    game.set_body(
        id,
        &BodyPatch {
            angles: Some(ZERO),
            origin: Some(vadd(
                origin,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: lift,
                },
            )),
            ..Default::default()
        },
    )
}

fn spawn_teleport_first(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    spawn_boss_teleport_point(game, id, 27.0)
}

fn spawn_teleport_second(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    spawn_boss_teleport_point(game, id, 27.0)
}

fn spawn_teleport_boss(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    spawn_boss_teleport_point(game, id, 0.0)
}

fn trigger_boss_teleport_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let variant = game.entity_ref(id).is_some_and(|entity| entity.spawnflags & 1 != 0);
    boss_teleport(game, variant)
}

fn spawn_trigger_boss_teleport(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let use_callback = game
        .named
        .use_callback(&format!("{FINAL_PREFIX}:trigger_boss_teleport_use"))?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_callback))
}

fn rock_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    let owner = game.entity_ref(id).and_then(|entity| entity.owner.clone());
    if owner.as_ref().is_some_and(|owner| same_actor(owner, other)) {
        return Ok(());
    }
    let classname = game.host.classname(other);
    if classname == "monster_orb" || classname == "monster_lava_man" {
        return game.remove(id);
    }
    let own_class = game
        .entity_ref(id)
        .map(|entity| entity.classname.clone())
        .unwrap_or_default();
    let trigger = game
        .entity_ref(other)
        .is_some_and(|entity| entity.solid == Q1Solid::Trigger);
    if classname == own_class || trigger {
        return Ok(());
    }
    if game.host.contents(game.body(id)?.origin) == Q1Contents::Sky {
        return game.remove(id);
    }
    if game
        .host
        .combat
        .read(other)
        .is_some_and(|combat| combat.can_take_damage)
    {
        let velocity = game.body(id)?.velocity;
        let up = game.basis.up;
        let right = game.basis.right;
        let jitter_up = game.host.random() - 0.5;
        let jitter_right = game.host.random() - 0.5;
        let direction = normalize(vadd(
            vadd(normalize(velocity), vscale(up, jitter_up)),
            vscale(right, jitter_right),
        ));
        let spray = vscale(vscale(vadd(direction, vscale(normal.unwrap_or(ZERO), 2.0)), 200.0), 0.2);
        game.host.emit(Q1Event::Particles {
            origin: vadd(game.body(id)?.origin, vscale(spray, 0.01)),
            direction: vscale(spray, 0.1),
            color: 73,
            count: 36,
        });
        let attacker = game.entity_ref(id).and_then(|entity| entity.owner.clone());
        game.damage(other, Some(id), attacker.as_ref(), 18.0, &Q1DamageParams::default());
    } else if game.entity_ref(id).map(|entity| entity.number("frags")).unwrap_or(0.0) != 0.0 {
        let origin = game.body(id)?.origin;
        game.effect(Q1Effect::KnightSpike, origin, None, 1);
    }
    game.remove(id)
}

fn rock_death(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let origin = game.body(id)?.origin;
    let ids = game.entity_ids();
    for other in ids {
        let rock = game.entity_ref(&other).is_some_and(|entity| entity.classname == "rock");
        if !rock {
            continue;
        }
        let distance = game
            .body(&other)
            .map(|body| f64::from(length(vsub(body.origin, origin))))
            .unwrap_or(f64::INFINITY);
        if distance <= 96.0 {
            later(game, &other, "Rock_Death", 0.1)?;
        }
    }
    game.effect(Q1Effect::Explosion, origin, None, 1);
    game.remove(id)
}

fn spiral_single(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let angles = game.body(id)?.angles;
    game.set_body(
        id,
        &BodyPatch {
            angles: Some(vadd(angles, Vec3 { x: 0.0, y: 5.0, z: 0.0 })),
            ..Default::default()
        },
    )?;
    let angles = game.body(id)?.angles;
    game.make_vectors(angles);
    let (count, delay) = game
        .entity_ref(id)
        .map(|entity| (entity.count, entity.delay))
        .unwrap_or((0.0, 0.0));
    if game.options().skill < 2 && count % 10.0 < 5.0 {
        later(game, id, "spiral_single", delay)?;
        game.update_entity(id, |entity| entity.count -= 1.0)?;
        return Ok(());
    }
    for i in (0..4).rev() {
        let angles = game.body(id)?.angles;
        let direction = game
            .make_vectors(vadd(
                angles,
                Vec3 {
                    x: 0.0,
                    y: (90.0 * f64::from(i)) as f32,
                    z: 0.0,
                },
            ))
            .forward;
        let owner = game.entity_ref(id).and_then(|entity| entity.owner.clone());
        let shot = fire_rock(
            game,
            owner.as_ref(),
            vadd(game.body(id)?.origin, vscale(direction, 100.0)),
            direction,
            "plasma",
        )?;
        game.set_body(
            &shot,
            &BodyPatch {
                velocity: Some(vscale(direction, 100.0)),
                ..Default::default()
            },
        )?;
        spin_rock(game, &shot)?;
        game.schedule(&shot, 30.0, "SUB_Remove")?;
    }
    game.update_entity(id, |entity| entity.count -= 1.0)?;
    let owner = game.entity_ref(id).and_then(|entity| entity.owner.clone());
    if owner.as_ref().is_none_or(|owner| game.health(owner) <= 0.0) {
        return game.remove(id);
    }
    later(game, id, "spiral_single", delay)
}

fn circle_think(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    for remaining in (1..=72).rev() {
        let direction = game
            .make_vectors(Vec3 {
                x: 0.0,
                y: (5.0 * f64::from(72 - remaining)) as f32,
                z: 0.0,
            })
            .forward;
        let shot = fire_rock(game, Some(id), game.body(id)?.origin, direction, "sphere")?;
        let signed = random_signed(game);
        game.set_body(
            &shot,
            &BodyPatch {
                velocity: Some(vscale(direction, 250.0 + signed * 100.0)),
                ..Default::default()
            },
        )?;
        spin_rock(game, &shot)?;
        game.schedule(&shot, 20.0, "SUB_Remove")?;
        if remaining % 4 == 0 {
            game.update_entity(&shot, |entity| entity.effects |= 8)?;
        }
    }
    game.remove(id)
}

/// Ends the boss fight (`bossEnd`).
fn boss_end(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    let first = (game.host.players)().into_iter().next();
    let alive = first.as_ref().is_some_and(|player| game.health(player) > 0.0);
    if !game.options().coop && !alive {
        return Ok(());
    }
    let players = (game.host.players)();
    for actor in players {
        let player = match game.player_owned(&actor) {
            Some(player) => player,
            None => continue,
        };
        for entry in game.host.inventory.entries(&actor) {
            if entry.item == "q1:ammo/shells"
                || entry.item == "q1:ammo/nails"
                || entry.item == "q1:ammo/cells"
                || entry.item == "q1:ammo/rockets"
            {
                let mut reset = entry.clone();
                reset.count = if entry.item == "q1:ammo/shells" { 25.0 } else { 0.0 };
                game.host.inventory.configure(&player, &reset)?;
            }
        }
        game.host.combat.set_armor(
            &player,
            &ArmorState {
                regular: RegularArmorState::None,
                powered: PoweredProtectionState::None,
            },
        )?;
        game.update_player(&actor, |state| state.weapon = Q1Weapon::Shotgun)?;
    }
    let flags = update_base(game, |state| state.campaign.read_flags())?;
    let mut map = "start";
    if flags & BLOODY_NIGHTMARE_ACTIVE != 0 {
        if flags & BLOODY_NIGHTMARE_NEWGAME != 0 {
            map = "boss2";
        } else {
            map = "map1";
            update_base(game, |state| {
                state
                    .campaign
                    .write_flags(BLOODY_NIGHTMARE_ACTIVE | BLOODY_NIGHTMARE_DISCOVERED | BLOODY_NIGHTMARE_NEWGAME);
            })?;
            if let Some(world) = game.world.clone() {
                game.update_entity(&world, |entity| {
                    entity
                        .fields
                        .insert(String::from("mg3.finalNewGameTravel"), String::from("1"));
                })?;
            }
            let players = (game.host.players)();
            for actor in players {
                for parm in ["parm10", "parm11", "parm12", "parm13", "parm14"] {
                    set_addon_player_number(game, &actor, parm, 0.0)?;
                }
            }
        }
    }
    if let Some(world) = game.world.clone() {
        game.update_entity(&world, |entity| {
            entity.fields.insert(
                String::from("addon.intermissiontext"),
                String::from("$mg3_qc_boss_finale"),
            );
        })?;
    }
    let mut rules = update_base(game, |state| state.level_rules.clone())?;
    rules.begin(game, map, None)?;
    let moved = rules.clone();
    update_base(game, |state| state.level_rules = moved)
}

fn boss_end_action(game: &mut Q1EntityServices, _id: &ActorId) -> Result<(), Q1Error> {
    boss_end(game)
}

/// Registers the final boss (`registerFinalBoss`).
///
/// Like the old one gate, the donor `mg3:final` damage-source stages
/// are not registered: the Rust stages receive no game access and
/// currently have no callers.
pub fn register_final_boss(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    register_mg3_monster_source(
        FINAL_PREFIX,
        frames(),
        final_actions(),
        Mg3SourceHooks {
            pain: Some(final_pain),
            die: Some(final_die),
            play: Some(final_play),
            ..Default::default()
        },
        final_load_controller,
        store_monster_controller,
    );
    register_boss_controllers(game, FINAL_PREFIX, "monster_boss_final", spawn_final)?;
    game.named.register(
        &format!("{FINAL_PREFIX}:final_awake"),
        Q1CallbackHandlers {
            use_callback: Some(final_awake_use as Q1UseHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{FINAL_PREFIX}:boss_final_melee_check"),
        Q1CallbackHandlers {
            action: Some(final_melee_action as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{FINAL_PREFIX}:boss_final_rise1"),
        Q1CallbackHandlers {
            action: Some(final_rise_action as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.register_spawn("info_boss_teleport_first", spawn_teleport_first)?;
    game.register_spawn("info_boss_teleport_second", spawn_teleport_second)?;
    game.register_spawn("info_boss_teleport_boss", spawn_teleport_boss)?;
    game.named.register(
        &format!("{FINAL_PREFIX}:trigger_boss_teleport_use"),
        Q1CallbackHandlers {
            use_callback: Some(trigger_boss_teleport_use as Q1UseHandler),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_boss_teleport", spawn_trigger_boss_teleport)?;
    game.named.register(
        &format!("{FINAL_PREFIX}:T_RockTouch2"),
        Q1CallbackHandlers {
            touch: Some(rock_touch as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{FINAL_PREFIX}:Rock_Death"),
        Q1CallbackHandlers {
            action: Some(rock_death as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{FINAL_PREFIX}:spiral_single"),
        Q1CallbackHandlers {
            action: Some(spiral_single as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{FINAL_PREFIX}:circle_think"),
        Q1CallbackHandlers {
            action: Some(circle_think as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        "mg3:bosses:boss_end",
        Q1CallbackHandlers {
            action: Some(boss_end_action as Q1ActionHandler),
            ..Default::default()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::{attach_test_player, register_test_addons, Q1AddonProgram};
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::missionpacks::types::test_game;

    fn setup(game: &mut Q1EntityServices) -> Q1BaseGuard {
        let guard = Q1BaseGuard::register(game, Q1BaseOptions::default()).expect("base");
        register_test_addons(game, Q1AddonProgram::Mg3);
        register_final_boss(game).expect("final");
        guard
    }

    #[test]
    fn spawn_renames_and_arms_boss() {
        let mut game = test_game();
        let _guard = setup(&mut game);
        let boss = game.create("monster_boss_final", None, None).expect("boss");
        game.spawn_entity(&boss, None).expect("spawn");
        let entity = game.entity_ref(&boss).expect("entity");
        assert_eq!(entity.classname, "monster_boss");
        assert_eq!(entity.number("boss_immune"), 1.0);
        assert_eq!(entity.number("frags"), 0.0);
        assert_eq!(game.total_monsters, 1);
    }

    #[test]
    fn awake_opens_boss_for_combat() {
        let mut game = test_game();
        let _guard = setup(&mut game);
        let boss = game.create("monster_boss_final", None, None).expect("boss");
        game.spawn_entity(&boss, None).expect("spawn");
        game.invoke_use(&boss, &format!("{FINAL_PREFIX}:final_awake"), None, None)
            .expect("awake");
        let entity = game.entity_ref(&boss).expect("entity");
        assert_eq!(entity.model, "progs/boss.mdl");
        assert_eq!(entity.max_health, 12000.0);
        assert!(entity.die.is_some() && entity.pain.is_some());
    }

    #[test]
    fn teleport_points_store_mangle() {
        let mut game = test_game();
        let _guard = setup(&mut game);
        let point = game.create("info_boss_teleport_first", None, None).expect("point");
        game.set_body(
            &point,
            &BodyPatch {
                origin: Some(Vec3 {
                    x: 10.0,
                    y: 20.0,
                    z: 30.0,
                }),
                ..Default::default()
            },
        )
        .expect("origin");
        game.spawn_entity(&point, None).expect("spawn");
        let entity = game.entity_ref(&point).expect("entity");
        assert_eq!(entity.model, "");
        let body = game.body(&point).expect("body");
        assert_eq!(body.origin.z, 57.0);
        assert_eq!(body.angles, ZERO);
    }

    #[test]
    fn boss_end_requires_live_player() {
        let mut game = test_game();
        let _guard = setup(&mut game);
        let player = attach_test_player(&mut game);
        game.host
            .inventory
            .configure(
                &game.player_owned(&player).expect("owned"),
                &crate::contract::InventoryEntry {
                    item: String::from("q1:ammo/shells"),
                    count: 100.0,
                    capacity: 100.0,
                    count_policy: None,
                },
            )
            .expect("shells");
        boss_end(&mut game).expect("end");
        let entries = game.host.inventory.entries(&player);
        let shells = entries
            .iter()
            .find(|entry| entry.item == "q1:ammo/shells")
            .expect("shells");
        assert_eq!(shells.count, 100.0);
    }
}
