//! Q1 mg3 new old one (`src/content/q1/addons/monsters/bosses/oldnew.ts`).
//!
//! `quakec_mg3/monsters/mg3_oldone_new.qc`. GPL-2.0-or-later.

use std::collections::HashMap;
use std::sync::OnceLock;

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

use crate::q1::addons::context::{
    addon_emit, set_addon_number, set_addon_vector, set_combat_team, Q1AddonContext, Q1AddonEvent, Q1AddonProgram,
};
use crate::q1::addons::monsters::ai::{register_mg3_monster_source, Mg3ActionHandler, Mg3Monster, Mg3SourceHooks};
use crate::q1::base::animation::MonsterAi;
use crate::q1::base::creatures::{monster_controller, store_monster_controller};
use crate::q1::base::monsters::BaseMonsterState;
use crate::q1::base::provider::update_base;
use crate::q1::base::species::{MonsterMovement, MonsterSpecies};
use crate::q1::foundation::callbacks::{Q1ActionHandler, Q1CallbackHandlers};
use crate::q1::foundation::entity::Q1MonsterSpecies;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::BodyPatch;
use crate::q1::foundation::types::{vadd, vscale, Q1Event, Q1MoveType, Q1Solid, Q1SoundChannel, POINT};
use crate::q1::Q1Error;

use super::effects::pain_lightning;
use super::frames::oldnew::frames;
use super::oldnew_children::{
    cleanup_oldnew, register_oldnew_children, spawn_blaster, spawn_eye, spawn_spammer, spawn_swiper, spawn_vortex,
};
use super::oldnew_projectiles::{
    auto_gun, register_oldnew_projectiles, spawn_sphere_chunk_manager, spawn_sphere_manager, OLDNEW_PREFIX,
};
use super::registry::register_boss_controllers;
use super::sphere_points::sphere_point;
use super::szombie::spawn_shub_zombie;

/// Old one callback prefix.
const OLDNEW_PREFIX_MONSTER: &str = "mg3:oldnew";

/// Old one spawn defaults (`spec`).
const OLDNEW_SPEC: MonsterSpecies = MonsterSpecies {
    species: Q1MonsterSpecies::Oldone,
    kill_string: None,
    classnames: &["monster_oldone_new"],
    model: "oldone",
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
    stand: "oldnew_idle1",
    walk: "oldnew_idle1",
    run: "oldnew_walk1",
    sight: "",
    missile: None,
    melee: false,
    movement: MonsterMovement::Walk,
};

fn mg3_context() -> Q1AddonContext {
    Q1AddonContext::new(Q1AddonProgram::Mg3)
}

/// Fires a target wave through the boss (`wave`).
fn oldnew_wave(monster: &mut Mg3Monster, field: &str) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    let target = monster
        .monster
        .game
        .entity_ref(&id)
        .map(|entity| entity.text(field))
        .unwrap_or_default();
    if target.is_empty() {
        return Ok(());
    }
    let previous = monster
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
        .update_entity(&id, |entity| entity.target = previous)
}

/// Runs the boss attack dispatch (`attack`).
fn oldnew_attack(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    let context = mg3_context();
    let immune = monster
        .monster
        .game
        .entity_ref(&id)
        .map(|entity| entity.number("boss_immune"))
        .unwrap_or(0.0);
    if immune != 0.0 {
        return Ok(());
    }
    let (phase, count, cnt) = monster
        .monster
        .game
        .entity_ref(&id)
        .map(|entity| (entity.number("boss_phase"), entity.count, entity.number("cnt")))
        .unwrap_or((0.0, 0.0, 0.0));
    if cnt % 3.0 != 0.0 {
        spawn_sphere_chunk_manager(
            &context,
            monster.monster.game,
            &id,
            if phase <= 3.0 { 1.0 } else { 2.0 },
        )?;
    } else if phase == 0.0 {
        spawn_sphere_chunk_manager(&context, monster.monster.game, &id, 1.0)?;
    } else {
        if phase == 1.0 {
            if count == 0.0 {
                spawn_spammer(&context, monster.monster.game, &id)?;
            } else if count == 1.0 {
                spawn_swiper(&context, monster.monster.game, &id)?;
            }
        } else if phase == 2.0 {
            if count == 0.0 {
                spawn_vortex(&context, monster.monster.game, &id)?;
            } else if count == 1.0 {
                spawn_swiper(&context, monster.monster.game, &id)?;
            } else if count == 2.0 {
                spawn_spammer(&context, monster.monster.game, &id)?;
            }
        } else if phase == 3.0 {
            if count == 0.0 {
                spawn_blaster(&context, monster.monster.game, &id)?;
            } else if (1.0..=3.0).contains(&count) {
                spawn_swiper(&context, monster.monster.game, &id)?;
            }
        } else if phase >= 4.0 {
            if count == 0.0 {
                spawn_spammer(&context, monster.monster.game, &id)?;
            } else if count == 1.0 {
                spawn_eye(&context, monster.monster.game, &id)?;
            } else if count == 2.0 {
                spawn_swiper(&context, monster.monster.game, &id)?;
            } else if count == 3.0 {
                spawn_blaster(&context, monster.monster.game, &id)?;
            }
        }
        let limit = if phase == 1.0 {
            1.0
        } else if phase == 2.0 {
            2.0
        } else {
            3.0
        };
        monster.monster.game.update_entity(&id, |entity| {
            entity.count += 1.0;
            if entity.count > limit {
                entity.count = 0.0;
            }
        })?;
    }
    set_addon_number(monster.monster.game, &id, "cnt", cnt + 1.0)
}

/// Fires one auto-gun volley (`autoGun`).
fn oldnew_auto_gun(monster: &mut Mg3Monster, side: f64, offset: f64, count: f64) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    let context = mg3_context();
    let (immune, phase) = monster
        .monster
        .game
        .entity_ref(&id)
        .map(|entity| (entity.number("boss_immune"), entity.number("boss_phase")))
        .unwrap_or((0.0, 0.0));
    let skill = monster.monster.game.options().skill;
    if immune != 0.0 || count > phase + 2.0 || skill == 0 || count > f64::from(skill + 1) {
        return Ok(());
    }
    let angles = monster.monster.game.body(&id)?.angles;
    let basis = monster.monster.game.make_vectors(angles);
    monster
        .monster
        .game
        .sound(&id, "weapons/spike2.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
    let origin = monster.monster.origin()?;
    auto_gun(
        &context,
        monster.monster.game,
        &id,
        vadd(
            vadd(
                origin,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 80.0,
                },
            ),
            vscale(basis.right, side * 64.0),
        ),
        offset,
    )
}

/// Old one frame actions (`actions`).
fn oldnew_actions() -> &'static HashMap<String, Mg3ActionHandler> {
    static ACTIONS: OnceLock<HashMap<String, Mg3ActionHandler>> = OnceLock::new();
    ACTIONS.get_or_init(|| {
        fn idle1(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.ai(MonsterAi::Stand, 0.0)
        }
        fn walk1(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.face()?;
            let id = monster.monster.id.clone();
            set_addon_number(monster.monster.game, &id, "boss_immune", 0.0)
        }
        fn walk2(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.face()
        }
        fn walk14(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.face()?;
            oldnew_attack(monster)
        }
        fn walk21(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.face()?;
            oldnew_auto_gun(monster, 1.0, 0.05, 1.0)
        }
        fn walk22(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.face()?;
            oldnew_auto_gun(monster, 1.0, 0.0, 2.0)
        }
        fn walk23(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.face()?;
            spawn_shub_zombie(monster.monster.game)?;
            oldnew_auto_gun(monster, 1.0, -0.05, 3.0)
        }
        fn walk24(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.face()?;
            oldnew_auto_gun(monster, 1.0, -0.1, 4.0)
        }
        fn walk25(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.face()?;
            oldnew_auto_gun(monster, 1.0, -0.15, 5.0)
        }
        fn walk36(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.face()?;
            oldnew_auto_gun(monster, -1.0, -0.05, 1.0)
        }
        fn walk37(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.face()?;
            oldnew_auto_gun(monster, -1.0, 0.0, 2.0)
        }
        fn walk38(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.face()?;
            oldnew_auto_gun(monster, -1.0, 0.05, 3.0)
        }
        fn walk39(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.face()?;
            oldnew_auto_gun(monster, -1.0, 0.1, 4.0)
        }
        fn walk40(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.face()?;
            oldnew_auto_gun(monster, -1.0, 0.15, 5.0)
        }
        fn walk45(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.face()?;
            spawn_shub_zombie(monster.monster.game)?;
            Ok(())
        }
        fn thrash4(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            let context = mg3_context();
            pain_lightning(
                &context,
                monster.monster.game,
                &id,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 100.0,
                },
            )
        }
        fn thrash14(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            let context = mg3_context();
            set_addon_number(monster.monster.game, &id, "boss_immune", 1.0)?;
            let phase = monster
                .monster
                .game
                .entity_ref(&id)
                .map(|entity| entity.number("boss_phase"))
                .unwrap_or(0.0);
            let skill = monster.monster.game.options().skill;
            spawn_sphere_manager(&context, monster.monster.game, &id, if skill > 2 { phase } else { 1.0 })?;
            Ok(())
        }
        fn thrash15(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            set_addon_number(monster.monster.game, &id, "boss_immune", 0.0)
        }
        fn death1(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            monster
                .monster
                .game
                .sound(&id, "boss2/death.wav", Q1SoundChannel::Voice, 1.0, 1.0)
        }
        fn death15(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            let cnt = monster
                .monster
                .game
                .entity_ref(&id)
                .map(|entity| entity.number("cnt"))
                .unwrap_or(0.0);
            set_addon_number(monster.monster.game, &id, "cnt", cnt + 1.0)?;
            if cnt + 1.0 != 3.0 {
                monster.monster.controller.next_frame = String::from("oldnew_death1");
            }
            Ok(())
        }
        fn death16(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.game.host.emit(Q1Event::Lightstyle {
                style: 0,
                pattern: String::from("g"),
            });
            Ok(())
        }
        fn death17(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.game.host.emit(Q1Event::Lightstyle {
                style: 0,
                pattern: String::from("c"),
            });
            Ok(())
        }
        fn death18(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.game.host.emit(Q1Event::Lightstyle {
                style: 0,
                pattern: String::from("b"),
            });
            Ok(())
        }
        fn death19(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.game.host.emit(Q1Event::Lightstyle {
                style: 0,
                pattern: String::from("a"),
            });
            Ok(())
        }
        fn death20(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            oldnew_finish(monster)
        }
        HashMap::from([
            (String::from("oldnew:oldnew_idle1"), idle1 as Mg3ActionHandler),
            (String::from("oldnew:oldnew_walk1"), walk1 as Mg3ActionHandler),
            (String::from("oldnew:oldnew_walk2"), walk2 as Mg3ActionHandler),
            (String::from("oldnew:oldnew_walk14"), walk14 as Mg3ActionHandler),
            (String::from("oldnew:oldnew_walk21"), walk21 as Mg3ActionHandler),
            (String::from("oldnew:oldnew_walk22"), walk22 as Mg3ActionHandler),
            (String::from("oldnew:oldnew_walk23"), walk23 as Mg3ActionHandler),
            (String::from("oldnew:oldnew_walk24"), walk24 as Mg3ActionHandler),
            (String::from("oldnew:oldnew_walk25"), walk25 as Mg3ActionHandler),
            (String::from("oldnew:oldnew_walk36"), walk36 as Mg3ActionHandler),
            (String::from("oldnew:oldnew_walk37"), walk37 as Mg3ActionHandler),
            (String::from("oldnew:oldnew_walk38"), walk38 as Mg3ActionHandler),
            (String::from("oldnew:oldnew_walk39"), walk39 as Mg3ActionHandler),
            (String::from("oldnew:oldnew_walk40"), walk40 as Mg3ActionHandler),
            (String::from("oldnew:oldnew_walk45"), walk45 as Mg3ActionHandler),
            (String::from("oldnew:oldnew_thrash4"), thrash4 as Mg3ActionHandler),
            (String::from("oldnew:oldnew_thrash14"), thrash14 as Mg3ActionHandler),
            (String::from("oldnew:oldnew_thrash15"), thrash15 as Mg3ActionHandler),
            (String::from("oldnew:oldnew_death1"), death1 as Mg3ActionHandler),
            (String::from("oldnew:oldnew_death15"), death15 as Mg3ActionHandler),
            (String::from("oldnew:oldnew_death16"), death16 as Mg3ActionHandler),
            (String::from("oldnew:oldnew_death17"), death17 as Mg3ActionHandler),
            (String::from("oldnew:oldnew_death18"), death18 as Mg3ActionHandler),
            (String::from("oldnew:oldnew_death19"), death19 as Mg3ActionHandler),
            (String::from("oldnew:oldnew_death20"), death20 as Mg3ActionHandler),
        ])
    })
}

fn oldnew_load_controller(
    game: &Q1EntityServices,
    id: &ActorId,
    _classname: &str,
) -> Result<(BaseMonsterState, &'static MonsterSpecies), Q1Error> {
    let controller = monster_controller(game, id, "monster_oldone_new")?;
    Ok((controller, &OLDNEW_SPEC))
}

fn oldnew_pain(monster: &mut Mg3Monster, attacker: Option<&ActorId>, _damage: f64) -> Result<(), Q1Error> {
    monster.retaliate(attacker)?;
    let id = monster.monster.id.clone();
    let context = mg3_context();
    let immune = monster
        .monster
        .game
        .entity_ref(&id)
        .map(|entity| entity.number("boss_immune"))
        .unwrap_or(0.0);
    if immune != 0.0 {
        return Ok(());
    }
    let mut trigger = false;
    let phase = monster
        .monster
        .game
        .entity_ref(&id)
        .map(|entity| entity.number("boss_phase"))
        .unwrap_or(0.0);
    if phase == 0.0 {
        trigger = true;
        set_addon_number(monster.monster.game, &id, "boss_phase", 1.0)?;
    }
    let health = monster.monster.game.health(&id);
    let max_health = monster
        .monster
        .game
        .entity_ref(&id)
        .map(|entity| entity.max_health)
        .unwrap_or(0.0);
    let phase = monster
        .monster
        .game
        .entity_ref(&id)
        .map(|entity| entity.number("boss_phase"))
        .unwrap_or(0.0);
    if phase == 1.0 && health < max_health * 0.75 {
        trigger = true;
        set_addon_number(monster.monster.game, &id, "boss_phase", 2.0)?;
        spawn_eye(&context, monster.monster.game, &id)?;
        oldnew_wave(monster, "wave1")?;
    }
    let phase = monster
        .monster
        .game
        .entity_ref(&id)
        .map(|entity| entity.number("boss_phase"))
        .unwrap_or(0.0);
    if phase == 2.0 && health < max_health * 0.5 {
        trigger = true;
        set_addon_number(monster.monster.game, &id, "boss_phase", 3.0)?;
        oldnew_wave(monster, "wave2")?;
    }
    let phase = monster
        .monster
        .game
        .entity_ref(&id)
        .map(|entity| entity.number("boss_phase"))
        .unwrap_or(0.0);
    if phase == 3.0 && health < max_health * 0.25 {
        trigger = true;
        set_addon_number(monster.monster.game, &id, "boss_phase", 4.0)?;
        oldnew_wave(monster, "wave2")?;
    }
    if !trigger {
        return Ok(());
    }
    set_addon_number(monster.monster.game, &id, "boss_immune", 1.0)?;
    monster
        .monster
        .game
        .sound(&id, "orb/orb_pain.wav", Q1SoundChannel::Voice, 1.0, 1.0)?;
    let time = monster.monster.game.time;
    monster.monster.monster.pain_finished = time + 2.1;
    monster.play("oldnew_thrash1")
}

fn oldnew_die(monster: &mut Mg3Monster, attacker: Option<&ActorId>) -> Result<(), Q1Error> {
    if monster.monster.controller.counted_death {
        return Ok(());
    }
    monster.monster.monster.enemy = attacker.cloned();
    monster
        .monster
        .game
        .set_damageable(&monster.monster.id.clone(), false)?;
    monster
        .monster
        .game
        .update_entity(&monster.monster.id.clone(), |entity| {
            entity.touch = None;
        })?;
    monster.monster.count_kill()?;
    let id = monster.monster.id.clone();
    let context = mg3_context();
    set_addon_number(monster.monster.game, &id, "boss_immune", 1.0)?;
    set_addon_number(monster.monster.game, &id, "cnt", 0.0)?;
    monster.play("oldnew_death1")?;
    cleanup_oldnew(&context, monster.monster.game)?;
    let phase = monster
        .monster
        .game
        .entity_ref(&id)
        .map(|entity| entity.number("boss_phase"))
        .unwrap_or(0.0);
    let skill = monster.monster.game.options().skill;
    spawn_sphere_manager(&context, monster.monster.game, &id, if skill > 2 { phase } else { 1.0 })?;
    Ok(())
}

/// Finishes the old one death sequence (`finish`).
fn oldnew_finish(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    monster
        .monster
        .game
        .sound(&id, "boss2/pop2.wav", Q1SoundChannel::Voice, 1.0, 1.0)?;
    let player = (monster.monster.game.host.players)().into_iter().next();
    let alive = player
        .as_ref()
        .is_some_and(|player| monster.monster.game.health(player) > 0.0);
    if !monster.monster.game.options().coop && !alive {
        return monster.play("oldnew_idle1");
    }
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
    monster.monster.game.remove(&id)?;
    addon_emit(
        monster.monster.game,
        Q1AddonEvent::Music {
            track: 3,
            loop_track: 3,
        },
    )?;
    monster.monster.game.host.emit(Q1Event::Lightstyle {
        style: 0,
        pattern: String::from("m"),
    });
    oldnew_credits(monster.monster.game)
}

fn spawn_oldnew(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let mut monster = Mg3Monster::spawn_new(game, OLDNEW_PREFIX_MONSTER, id, &OLDNEW_SPEC)?;
    if monster.monster.game.options().deathmatch != 0 {
        let id = monster.monster.id.clone();
        monster.monster.game.remove(&id)?;
        return monster.finish();
    }
    let id = monster.monster.id.clone();
    let tiny = monster
        .monster
        .game
        .entity_ref(&id)
        .is_some_and(|entity| entity.spawnflags & 128 != 0);
    monster.monster.game.update_entity(&id, |entity| {
        entity.solid = Q1Solid::Slidebox;
        entity.movement = Q1MoveType::Step;
        entity.model = String::from("progs/oldone.mdl");
    })?;
    monster.monster.game.set_bounds(
        &id,
        if tiny {
            Bounds {
                min: Vec3 {
                    x: 1.0,
                    y: 1.0,
                    z: -24.0,
                },
                max: Vec3 { x: 1.0, y: 1.0, z: 0.0 },
            }
        } else {
            OLDNEW_SPEC.bounds
        },
    )?;
    let owned = monster.monster.game.entity_ref(&id).map(|entity| entity.actor.clone());
    if let Some(owned) = owned {
        monster.monster.game.host.combat.set_health(&owned, 12000.0)?;
        set_combat_team(monster.monster.game, &owned, Some("q1:monsters"))?;
    }
    monster.monster.game.update_entity(&id, |entity| {
        entity.max_health = 12000.0;
        entity.movement_flags = 32;
    })?;
    monster.monster.game.set_damageable(&id, true)?;
    monster.monster.game.update_entity(&id, |entity| {
        entity.aimed_damage = true;
    })?;
    set_addon_vector(
        monster.monster.game,
        &id,
        "view_ofs",
        Vec3 {
            x: 0.0,
            y: 0.0,
            z: 24.0,
        },
    )?;
    let pain = monster
        .monster
        .game
        .named
        .pain(&format!("{OLDNEW_PREFIX_MONSTER}:monster_pain"))?;
    let die = monster
        .monster
        .game
        .named
        .die(&format!("{OLDNEW_PREFIX_MONSTER}:monster_die"))?;
    let use_callback = monster
        .monster
        .game
        .named
        .use_callback(&format!("{OLDNEW_PREFIX_MONSTER}:monster_use"))?;
    let angles = monster.monster.game.body(&id)?.angles;
    monster.monster.game.update_entity(&id, |entity| {
        entity.pain = Some(pain);
        entity.die = Some(die);
        entity.use_callback = Some(use_callback);
        entity.yaw_speed = 10.0;
        entity.ideal_yaw = f64::from(angles.y);
    })?;
    monster.monster.game.total_monsters += 1;
    let total = monster.monster.game.total_monsters;
    monster.monster.game.host.emit(Q1Event::MonsterTotal { total });
    monster.monster.controller.next_frame = String::from(OLDNEW_SPEC.stand);
    monster.monster.delay(0.1)?;
    monster.finish()
}

/// Throws a directed gib (`throwGibVector`).
pub fn throw_gib_vector(game: &mut Q1EntityServices, origin: Vec3, direction: Vec3) -> Result<ActorId, Q1Error> {
    let gib = game.create("gib_vector", None, None)?;
    game.update_entity(&gib, |entity| entity.classname = String::new())?;
    let choice = game.host.random();
    let model = if choice < 0.3 {
        1
    } else if choice < 0.6 {
        2
    } else {
        3
    };
    game.update_entity(&gib, |entity| {
        entity.model = format!("progs/gib{model}.mdl");
    })?;
    let speed = 800.0 + (game.host.random() * 2.0 - 1.0) * 200.0;
    game.set_body(
        &gib,
        &BodyPatch {
            origin: Some(origin),
            velocity: Some(vscale(direction, speed)),
            bounds: Some(POINT),
            ..Default::default()
        },
    )?;
    let spin_x = game.host.random() * 600.0;
    let spin_y = game.host.random() * 600.0;
    let spin_z = game.host.random() * 600.0;
    game.update_entity(&gib, |entity| {
        entity.movement = Q1MoveType::Bounce;
        entity.solid = Q1Solid::None;
        entity.angular_velocity = Vec3 {
            x: spin_x as f32,
            y: spin_y as f32,
            z: spin_z as f32,
        };
    })?;
    let time = game.time;
    set_addon_number(game, &gib, "ltime", time)?;
    let lifetime = 10.0 + game.host.random() * 10.0;
    game.schedule(&gib, lifetime, "SUB_Remove")?;
    game.link(&gib)?;
    Ok(gib)
}

/// Starts the old one end credits (`oldnewCredits`).
pub fn oldnew_credits(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    if let Some(world) = game.world.clone() {
        game.update_entity(&world, |entity| {
            entity.fields.insert(
                String::from("addon.intermissiontext"),
                String::from("$map_dopa_endtext_final"),
            );
        })?;
    }
    let mut rules = update_base(game, |state| state.level_rules.clone())?;
    rules.begin(game, "start", None)?;
    let moved = rules.clone();
    update_base(game, |state| state.level_rules = moved)
}

fn oldnew_credits_action(game: &mut Q1EntityServices, _id: &ActorId) -> Result<(), Q1Error> {
    oldnew_credits(game)
}

/// Registers the new old one (`registerOldnew`).
///
/// The donor `mg3:oldone` damage-source gate is not registered: the
/// Rust damage stage receives no game access (it cannot read
/// `boss_immune`) and `before_health` currently has no callers, so
/// the immunity state is tracked but enforced engine-side.
pub fn register_oldnew(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    let context = mg3_context();
    register_oldnew_projectiles(&context, game)?;
    register_oldnew_children(&context, game)?;
    register_mg3_monster_source(
        OLDNEW_PREFIX_MONSTER,
        frames(),
        oldnew_actions(),
        Mg3SourceHooks {
            pain: Some(oldnew_pain),
            die: Some(oldnew_die),
            ..Default::default()
        },
        oldnew_load_controller,
        store_monster_controller,
    );
    register_boss_controllers(game, OLDNEW_PREFIX_MONSTER, "monster_oldone_new", spawn_oldnew)?;
    game.named.register(
        &format!("{OLDNEW_PREFIX}oldnew_credits"),
        Q1CallbackHandlers {
            action: Some(oldnew_credits_action as Q1ActionHandler),
            ..Default::default()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::{attach_test_player, register_test_addons};
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::foundation::types::ZERO;
    use crate::q1::missionpacks::types::test_game;

    fn setup(game: &mut Q1EntityServices) -> Q1BaseGuard {
        let guard = Q1BaseGuard::register(game, Q1BaseOptions::default()).expect("base");
        register_test_addons(game, Q1AddonProgram::Mg3);
        register_oldnew(game).expect("oldnew");
        guard
    }

    #[test]
    fn spawn_sets_oldnew_defaults() {
        let mut game = test_game();
        let _guard = setup(&mut game);
        let boss = game.create("monster_oldone_new", None, None).expect("boss");
        game.spawn_entity(&boss, None).expect("spawn");
        let entity = game.entity_ref(&boss).expect("entity");
        assert_eq!(entity.model, "progs/oldone.mdl");
        assert_eq!(entity.max_health, 12000.0);
        assert_eq!(entity.movement_flags, 32);
        assert_eq!(game.total_monsters, 1);
    }

    #[test]
    fn pain_advances_boss_phases() {
        let mut game = test_game();
        let _guard = setup(&mut game);
        let _player = attach_test_player(&mut game);
        let boss = game.create("monster_oldone_new", None, None).expect("boss");
        game.spawn_entity(&boss, None).expect("spawn");
        let mut monster = Mg3Monster::load(&mut game, &boss).expect("load");
        oldnew_pain(&mut monster, None, 10.0).expect("pain");
        monster.finish().expect("finish");
        let entity = game.entity_ref(&boss).expect("entity");
        assert_eq!(entity.number("boss_phase"), 1.0);
        assert_eq!(entity.number("boss_immune"), 1.0);
    }

    #[test]
    fn gib_vector_throws_directed_gibs() {
        let mut game = test_game();
        let _guard = setup(&mut game);
        let gib = throw_gib_vector(&mut game, ZERO, Vec3 { x: 0.0, y: 0.0, z: 1.0 }).expect("gib");
        let entity = game.entity_ref(&gib).expect("entity");
        assert!(entity.model.starts_with("progs/gib"));
        assert_eq!(entity.movement, Q1MoveType::Bounce);
        let body = game.body(&gib).expect("body");
        assert!(body.velocity.z > 0.0);
    }
}
