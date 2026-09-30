//! Q1 mg3 super shambler (`src/content/q1/addons/monsters/heavy/super-shambler.ts`).
//!
//! `quakec_mg3/monsters/mg3_super_shambler.qc`. GPL-2.0-or-later.

use std::collections::HashMap;
use std::sync::OnceLock;

use qa_core::identity::{same_actor, ActorId};
use qa_core::math::{Bounds, Vec3};

use crate::q1::addons::campaign::BLOODY_NIGHTMARE_ACTIVE;
use crate::q1::addons::context::set_addon_number;
use crate::q1::base::animation::MonsterAi;
use crate::q1::base::projectiles::{spawn_meat_spray, throw_gib, throw_head};
use crate::q1::base::provider::campaign_read_flags;
use crate::q1::base::species::{MonsterMovement, MonsterSpecies};
use crate::q1::foundation::entity::Q1MonsterSpecies;
use crate::q1::foundation::gameplay::BodyPatch;
use crate::q1::foundation::types::{
    dot, length, normalize, vadd, vscale, vsub, Q1BeamStyle, Q1Event, Q1SoundChannel, Q1TraceRequest, Q1Weapon, POINT,
    ZERO,
};
use crate::q1::Q1Error;

use super::projectiles::{heavy_lightning_damage, heavy_spike};
use super::runtime::{heavy_initialize, HEAVY_PREFIX};
use crate::q1::addons::monsters::ai::{Mg3ActionHandler, Mg3Monster};
use crate::q1::missionpacks::types::velocity_angles;

/// Super-shambler spawn defaults (`superShamblerDefinition.spec`).
pub const SUPER_SHAMBLER_SPEC: MonsterSpecies = MonsterSpecies {
    species: Q1MonsterSpecies::Shambler,
    kill_string: None,
    classnames: &["monster_super_shambler"],
    model: "shambler_blood",
    head: Some("h_shams"),
    health: 2000.0,
    gib_health: -60.0,
    gibs: &["gib1", "gib2", "gib3"],
    bounds: Bounds {
        min: Vec3 {
            x: -32.0,
            y: -32.0,
            z: -24.0,
        },
        max: Vec3 {
            x: 32.0,
            y: 32.0,
            z: 64.0,
        },
    },
    stand: "supsham_stand1",
    walk: "supsham_walk1",
    run: "supsham_run1",
    sight: "",
    missile: Some("supsham_melee"),
    melee: true,
    movement: MonsterMovement::Walk,
};

fn child_id(monster: &Mg3Monster) -> Option<ActorId> {
    monster
        .monster
        .game
        .entity_ref(&monster.monster.id.clone())
        .and_then(|entity| entity.references.get("child").and_then(|child| child.clone()))
}

fn live_child(monster: &Mg3Monster) -> Option<ActorId> {
    let child = child_id(monster)?;
    let entity = monster.monster.game.entity_ref(&child)?;
    if entity
        .owner
        .as_ref()
        .is_some_and(|owner| same_actor(owner, &monster.monster.id.clone()))
        && entity.classname == "lightning_child"
    {
        Some(child)
    } else {
        None
    }
}

fn remove_child(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    if let Some(child) = live_child(monster) {
        monster.monster.game.remove(&child)?;
    }
    monster
        .monster
        .game
        .update_entity(&monster.monster.id.clone(), |entity| {
            entity.references.insert(String::from("child"), None);
        })
}

fn child_frame(monster: &mut Mg3Monster, frame: i32) -> Result<(), Q1Error> {
    monster
        .monster
        .game
        .update_entity(&monster.monster.id.clone(), |entity| {
            entity.effects |= 2;
        })?;
    if let Some(child) = live_child(monster) {
        monster.monster.game.update_entity(&child, |entity| {
            entity.frame = frame;
        })?;
    }
    Ok(())
}

fn lightning_child(monster: &mut Mg3Monster, fast: bool) -> Result<(), Q1Error> {
    monster.monster.face()?;
    let id = monster.monster.id.clone();
    let delay = monster
        .monster
        .game
        .entity_ref(&id)
        .map(|entity| entity.next_think)
        .unwrap_or(0.0)
        - monster.monster.game.time
        + 0.2;
    monster.monster.delay(delay)?;
    monster.monster.game.update_entity(&id, |entity| {
        entity.effects |= 2;
    })?;
    monster.monster.face()?;
    let child = monster
        .monster
        .game
        .create(if fast { "" } else { "lightning_child" }, None, None)?;
    monster.monster.game.update_entity(&id, |entity| {
        entity.references.insert(String::from("child"), Some(child.clone()));
    })?;
    if !fast {
        monster.monster.game.update_entity(&child, |entity| {
            entity.owner = Some(id.clone());
        })?;
    }
    let origin = monster.monster.origin()?;
    let angles = monster.monster.game.body(&id).map(|body| body.angles)?;
    monster.monster.game.update_entity(&child, |entity| {
        entity.model = String::from("progs/s_light.mdl");
    })?;
    monster.monster.game.set_body(
        &child,
        &BodyPatch {
            origin: Some(origin),
            angles: Some(angles),
            ..Default::default()
        },
    )?;
    monster.monster.game.link(&child)?;
    let remove = monster.monster.game.named.action("SUB_Remove")?;
    monster
        .monster
        .game
        .schedule(&child, if fast { 0.7 } else { 1.4 }, &remove)
}

fn cast_lightning(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    monster.monster.game.update_entity(&id, |entity| {
        entity.effects |= 2;
    })?;
    monster.monster.face()?;
    let feet = monster.monster.origin()?;
    let target = monster.monster.target()?.unwrap_or(ZERO);
    let origin = vadd(
        feet,
        Vec3 {
            x: 0.0,
            y: 0.0,
            z: 40.0,
        },
    );
    let delta = vsub(
        vadd(
            target,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 16.0,
            },
        ),
        origin,
    );
    let range = f64::from(length(delta)).clamp(600.0, 1000.0);
    let direction = normalize(delta);
    let trace = monster.monster.game.host.trace(&Q1TraceRequest {
        start: origin,
        end: vadd(feet, vscale(direction, range)),
        bounds: POINT,
        ignore: Some(id.clone()),
        monsters: false,
        missile: false,
    });
    monster.monster.game.host.emit(Q1Event::Beam {
        style: Q1BeamStyle::Lightning1,
        actor: id.clone(),
        start: origin,
        end: trace.end,
    });
    let frags = monster
        .monster
        .game
        .entity_ref(&id)
        .map(|entity| entity.number("frags"))
        .unwrap_or(0.0);
    set_addon_number(monster.monster.game, &id, "frags", frags + 1.0)?;
    heavy_lightning_damage(monster.monster.game, &id, origin, trace.end, 10.0)
}

fn attack_check(monster: &mut Mg3Monster) -> Result<bool, Q1Error> {
    let mut cap = 3 + (monster.monster.game.host.random() * 2.0 + 0.5).floor() as i32;
    if campaign_read_flags(monster.monster.game)? & BLOODY_NIGHTMARE_ACTIVE != 0 {
        cap -= 1;
    }
    let id = monster.monster.id.clone();
    monster.monster.game.update_entity(&id, |entity| {
        entity.count += 1.0;
    })?;
    Ok(monster
        .monster
        .game
        .entity_ref(&id)
        .map(|entity| entity.count)
        .unwrap_or(0.0)
        > cap as f64)
}

fn z_offset(monster: &mut Mg3Monster) -> Result<f64, Q1Error> {
    let target = monster.monster.target()?.unwrap_or(ZERO);
    let origin = monster.monster.origin()?;
    let delta = vsub(target, origin);
    let random = monster.monster.game.host.random() * 2.0 - 1.0;
    if delta.z > 30.0 {
        return Ok(100.0 + random * 50.0);
    }
    if delta.z < -30.0 {
        return Ok(-100.0 + random * 50.0);
    }
    let range = f64::from(length(delta));
    if range > 300.0 {
        Ok(50.0 + random * 25.0)
    } else if range < 150.0 {
        Ok(-50.0 + random * 25.0)
    } else {
        Ok(0.0)
    }
}

fn shot(monster: &mut Mg3Monster, offset_y: f64, offset_z: f64, offset: Vec3, hands: bool) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    let target = monster.monster.target()?.unwrap_or(ZERO);
    let origin = monster.monster.origin()?;
    let body = monster.monster.game.body(&id)?;
    let mut angles = velocity_angles(vsub(target, origin));
    if hands {
        angles = Vec3 {
            x: angles.x,
            y: angles.y + offset_y as f32 * (12.0 + (monster.monster.game.host.random() * 2.0 - 1.0) as f32 * 4.0),
            z: angles.z,
        };
        angles = Vec3 {
            x: angles.x + offset_z as f32 * (12.0 + (monster.monster.game.host.random() * 2.0 - 1.0) as f32 * 4.0),
            y: angles.y,
            z: angles.z,
        };
    } else {
        angles = Vec3 {
            x: angles.x,
            y: angles.y + offset_y as f32 * 5.0,
            z: angles.z,
        };
    }
    let forward = monster.monster.game.make_vectors(angles).forward;
    let muzzle = vadd(
        vadd(
            vadd(body.origin, vscale(vadd(body.bounds.min, body.bounds.max), 0.5)),
            vscale(forward, 20.0),
        ),
        offset,
    );
    let mut direction = normalize(forward);
    if !hands {
        direction = Vec3 {
            x: direction.x,
            y: direction.y,
            z: -direction.z + (monster.monster.game.host.random() - 0.5) as f32 * 0.1,
        };
    }
    let missile = heavy_spike(monster.monster.game, &id, muzzle, vscale(direction, 1000.0))?;
    let spin = Vec3 {
        x: (300.0 * (monster.monster.game.host.random() * 2.0 - 1.0)) as f32,
        y: (300.0 * (monster.monster.game.host.random() * 2.0 - 1.0)) as f32,
        z: (300.0 * (monster.monster.game.host.random() * 2.0 - 1.0)) as f32,
    };
    monster.monster.game.update_entity(&missile, |entity| {
        entity.model = String::from("progs/rogue/plasma.mdl");
        entity.angular_velocity = spin;
        entity.movement = crate::q1::foundation::types::Q1MoveType::Toss;
    })?;
    monster.monster.game.set_bounds(&missile, POINT)?;
    let speed = monster.monster.game.host.random() * 100.0 + if hands { 500.0 } else { 400.0 };
    let vertical = (if hands { 100.0 } else { 125.0 }) + monster.monster.game.host.random() * 50.0;
    let mut velocity = vscale(direction, speed);
    velocity = Vec3 {
        x: velocity.x,
        y: velocity.y,
        z: vertical as f32,
    };
    let lift = z_offset(monster)?;
    velocity = Vec3 {
        x: velocity.x,
        y: velocity.y,
        z: velocity.z + lift as f32,
    };
    monster.monster.game.set_body(
        &missile,
        &BodyPatch {
            velocity: Some(velocity),
            ..Default::default()
        },
    )?;
    monster.monster.game.update_entity(&missile, |entity| {
        entity.effects |= 64;
    })?;
    monster.monster.game.link(&missile)
}

fn blast(monster: &mut Mg3Monster, offset: Vec3, hands: bool) -> Result<(), Q1Error> {
    if hands {
        for x in -1..2 {
            for y in -1..2 {
                shot(monster, f64::from(x), f64::from(y), offset, true)?;
            }
        }
    } else {
        for i in -5..=5 {
            shot(monster, f64::from(i), 0.0, offset, false)?;
        }
        monster.monster.game.sound(
            &monster.monster.id.clone(),
            "zombie/z_shot1.wav",
            Q1SoundChannel::Weapon,
            1.0,
            1.0,
        )?;
    }
    Ok(())
}

fn claw(monster: &mut Mg3Monster, side: f64) -> Result<(), Q1Error> {
    if monster.monster.monster.enemy.is_none() {
        return Ok(());
    }
    monster.ai(MonsterAi::Charge, 10.0)?;
    if monster.monster.distance()? > 100.0 {
        return Ok(());
    }
    monster.monster.melee(100.0, 20.0, 3, false)?;
    monster
        .monster
        .game
        .sound_simple(&monster.monster.id.clone(), "shambler/smack.wav")?;
    if side != 0.0 {
        let basis = monster.make_vectors()?;
        let origin = monster.monster.origin()?;
        spawn_meat_spray(
            monster.monster.game,
            &monster.monster.id.clone(),
            vadd(origin, vscale(basis.forward, 16.0)),
            vscale(basis.right, side),
        )?;
    }
    Ok(())
}

fn smash(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    blast(monster, ZERO, false)?;
    let Some(enemy) = monster.monster.monster.enemy.clone() else {
        return Ok(());
    };
    monster.ai(MonsterAi::Charge, 0.0)?;
    if monster.monster.distance()? > 100.0 || !monster.monster.game.can_damage(&enemy, &monster.monster.id.clone()) {
        return Ok(());
    }
    monster.monster.melee(100.0, 40.0, 3, false)?;
    monster
        .monster
        .game
        .sound_simple(&monster.monster.id.clone(), "shambler/smack.wav")?;
    for _ in 0..2 {
        let basis = monster.monster.game.basis;
        let origin = monster.monster.origin()?;
        let lateral = (monster.monster.game.host.random() * 2.0 - 1.0) * 100.0;
        spawn_meat_spray(
            monster.monster.game,
            &monster.monster.id.clone(),
            vadd(origin, vscale(basis.forward, 16.0)),
            vscale(basis.right, lateral),
        )?;
    }
    Ok(())
}

fn shambler_melee(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    let target = monster.monster.target()?.unwrap_or(ZERO);
    let origin = monster.monster.origin()?;
    let range = f64::from(length(vsub(origin, target)));
    if range < 110.0 {
        let first = monster.monster.game.host.random();
        if first < 0.8 {
            return monster.play("supsham_smash1");
        }
        let rolled = monster.monster.game.host.random();
        return monster.play(if rolled < 0.5 {
            "supsham_swingl1"
        } else {
            "supsham_swingr1"
        });
    }
    let id = monster.monster.id.clone();
    if range < 200.0
        || monster
            .monster
            .game
            .entity_ref(&id)
            .map(|entity| entity.wait)
            .unwrap_or(0.0)
            > monster.monster.game.time
    {
        let basis = monster.make_vectors()?;
        let delta = vsub(origin, target);
        let chance = f64::from(dot(basis.right, normalize(delta)));
        return monster.play(if chance < 0.45 {
            "supsham_swingr1"
        } else if chance < 0.9 {
            "supsham_swingl1"
        } else {
            "supsham_smash1"
        });
    }
    monster.monster.game.update_entity(&id, |entity| {
        entity.count = 0.0;
    })?;
    if monster.monster.game.host.random() > 0.4 {
        monster.play("supsham_magic1")?;
        let wait = monster.monster.game.time + 5.0;
        monster.monster.game.update_entity(&id, |entity| {
            entity.wait = wait;
        })?;
    } else {
        monster.play("supsham_magic_b1")?;
        let wait = monster.monster.game.time + 3.0;
        monster.monster.game.update_entity(&id, |entity| {
            entity.wait = wait;
        })?;
    }
    Ok(())
}

fn cleanup_orbs(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    for candidate in monster.monster.game.entity_ids() {
        let owned = monster
            .monster
            .game
            .entity_ref(&candidate)
            .filter(|entity| {
                entity.classname == "monster_super_shambler"
                    && entity.owner.as_ref().is_some_and(|owner| same_actor(owner, &id))
            })
            .is_some();
        if owned {
            let die = monster
                .monster
                .game
                .named
                .action(&format!("{HEAVY_PREFIX}:source_die"))?;
            monster.monster.game.schedule(&candidate, 0.1, &die)?;
        }
    }
    Ok(())
}

/// Super-shambler frame actions (`superShamblerDefinition.actions`).
pub fn super_shambler_actions() -> &'static HashMap<String, Mg3ActionHandler> {
    static ACTIONS: OnceLock<HashMap<String, Mg3ActionHandler>> = OnceLock::new();
    ACTIONS.get_or_init(|| {
        fn missile(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let rolled = monster.monster.game.host.random();
            monster.play(if rolled < 0.36 {
                "supsham_magic1"
            } else if rolled < 0.66 {
                "supsham_swingr1"
            } else {
                "supsham_swingl1"
            })
        }
        fn smash12(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.ai(MonsterAi::Charge, 4.0)?;
            if attack_check(monster)? {
                monster.monster.controller.next_frame = String::from("supsham_magic1");
            }
            Ok(())
        }
        fn swingl7(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let right = monster.monster.game.basis.right;
            blast(
                monster,
                vadd(
                    vscale(right, 16.0),
                    Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 32.0,
                    },
                ),
                true,
            )?;
            monster.ai(MonsterAi::Charge, 5.0)?;
            claw(monster, 250.0)
        }
        fn swingr7(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let right = monster.monster.game.basis.right;
            blast(
                monster,
                vadd(
                    vscale(right, -16.0),
                    Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 32.0,
                    },
                ),
                true,
            )?;
            monster.ai(MonsterAi::Charge, 6.0)?;
            claw(monster, -250.0)
        }
        fn swingl9(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.ai(MonsterAi::Charge, 8.0)?;
            monster.make_vectors()?;
            if attack_check(monster)? {
                monster.monster.controller.next_frame = String::from("supsham_magic1");
            } else if monster.monster.game.host.random() < 0.5 {
                monster.monster.controller.next_frame = String::from("supsham_swingr1");
            }
            Ok(())
        }
        fn swingr9(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.ai(MonsterAi::Charge, 1.0)?;
            monster.ai(MonsterAi::Charge, 10.0)?;
            monster.make_vectors()?;
            if attack_check(monster)? {
                monster.monster.controller.next_frame = String::from("supsham_magic1");
            } else if monster.monster.game.host.random() < 0.5 {
                monster.monster.controller.next_frame = String::from("supsham_swingl1");
            }
            Ok(())
        }
        fn magic1(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.face()?;
            monster.monster.game.sound(
                &monster.monster.id.clone(),
                "shambler/sattck1.wav",
                Q1SoundChannel::Weapon,
                1.0,
                1.0,
            )?;
            monster
                .monster
                .game
                .update_entity(&monster.monster.id.clone(), |entity| {
                    entity.count = 0.0;
                })
        }
        fn magic_slow(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            lightning_child(monster, false)
        }
        fn magic_fast(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            lightning_child(monster, true)
        }
        fn magic4(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            child_frame(monster, 1)
        }
        fn magic5(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            child_frame(monster, 2)
        }
        fn magic4b(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            child_frame(monster, 3)
        }
        HashMap::from([
            (String::from("supsham_removechild"), remove_child as Mg3ActionHandler),
            (String::from("SupCastLightning"), cast_lightning as Mg3ActionHandler),
            (String::from("supsham_melee"), shambler_melee as Mg3ActionHandler),
            (String::from("supsham_missile"), missile as Mg3ActionHandler),
            (String::from("cleanup_orbs"), cleanup_orbs as Mg3ActionHandler),
            (
                String::from("mg3_super_shambler:supsham_smash10"),
                smash as Mg3ActionHandler,
            ),
            (
                String::from("mg3_super_shambler:supsham_smash12"),
                smash12 as Mg3ActionHandler,
            ),
            (
                String::from("mg3_super_shambler:supsham_swingl7"),
                swingl7 as Mg3ActionHandler,
            ),
            (
                String::from("mg3_super_shambler:supsham_swingr7"),
                swingr7 as Mg3ActionHandler,
            ),
            (
                String::from("mg3_super_shambler:supsham_swingl9"),
                swingl9 as Mg3ActionHandler,
            ),
            (
                String::from("mg3_super_shambler:supsham_swingr9"),
                swingr9 as Mg3ActionHandler,
            ),
            (
                String::from("mg3_super_shambler:supsham_magic1"),
                magic1 as Mg3ActionHandler,
            ),
            (
                String::from("mg3_super_shambler:supsham_magic3"),
                magic_slow as Mg3ActionHandler,
            ),
            (
                String::from("mg3_super_shambler:supsham_magic_b3"),
                magic_fast as Mg3ActionHandler,
            ),
            (
                String::from("mg3_super_shambler:supsham_magic4"),
                magic4 as Mg3ActionHandler,
            ),
            (
                String::from("mg3_super_shambler:supsham_magic5"),
                magic5 as Mg3ActionHandler,
            ),
            (
                String::from("mg3_super_shambler:supsham_magic4b"),
                magic4b as Mg3ActionHandler,
            ),
        ])
    })
}

/// Spawn a super shambler (`superShamblerDefinition.spawn`).
pub fn super_shambler_spawn(
    monster: &mut Mg3Monster,
    context: &crate::q1::addons::context::Q1AddonContext,
) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    monster.monster.game.set_health(&id, 2000.0)?;
    set_addon_number(monster.monster.game, &id, "allowPathFind", 1.0)?;
    set_addon_number(monster.monster.game, &id, "combat_style", 3.0)?;
    set_addon_number(monster.monster.game, &id, "frags", 0.0)?;
    heavy_initialize(monster, context, 2)
}

/// Super-shambler melee (`superShamblerDefinition.melee`).
pub fn super_shambler_melee(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    shambler_melee(monster)
}

/// React to super-shambler pain (`superShamblerDefinition.pain`).
pub fn super_shambler_pain(monster: &mut Mg3Monster, attacker: Option<&ActorId>, damage: f64) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    let health = monster.monster.game.health(&id);
    monster.monster.game.sound_simple(&id, "shambler/shurt2.wav")?;
    if damage >= health {
        if let Some(attacker) = attacker {
            if monster.monster.game.is_player(attacker) {
                let axe = monster
                    .monster
                    .game
                    .player_ref(attacker)
                    .is_some_and(|player| player.weapon == Q1Weapon::Axe);
                if axe {
                    monster.monster.game.host.emit(Q1Event::Achievement {
                        player: Some(attacker.clone()),
                        id: String::from("ACH_CLOSE_SHAVE"),
                    });
                }
                if monster
                    .monster
                    .game
                    .entity_ref(&id)
                    .map(|entity| entity.number("frags"))
                    .unwrap_or(0.0)
                    == 0.0
                {
                    monster.monster.game.host.emit(Q1Event::Achievement {
                        player: Some(attacker.clone()),
                        id: String::from("ACH_SHAMBLER_DANCE"),
                    });
                }
            }
        }
    }
    if health <= 0.0
        || 25.0 + monster.monster.game.host.random() * 400.0 > damage
        || monster.monster.monster.pain_finished > monster.monster.game.time
    {
        return Ok(());
    }
    monster.monster.monster.pain_finished = monster.monster.game.time + 5.0;
    remove_child(monster)?;
    monster.play("supsham_pain1")
}

/// Die as a super shambler (`superShamblerDefinition.die`).
pub fn super_shambler_die(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    remove_child(monster)?;
    let health = monster.monster.game.health(&id);
    if health < -60.0 {
        monster.monster.game.sound_simple(&id, "player/udeath.wav")?;
        throw_head(monster.monster.game, &id, "h_shams", health)?;
        for model in ["gib1", "gib2", "gib3"] {
            let origin = monster.monster.origin()?;
            throw_gib(monster.monster.game, origin, model, health)?;
        }
        return Ok(());
    }
    monster.monster.game.sound_simple(&id, "shambler/sdeath.wav")?;
    monster.play("supsham_death1")
}
