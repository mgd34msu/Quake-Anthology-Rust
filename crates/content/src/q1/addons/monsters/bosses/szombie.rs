//! Q1 mg3 Shub zombie (`src/content/q1/addons/monsters/bosses/szombie.ts`).
//!
//! `quakec_mg3/monsters/mg3_shub_zombie.qc`. GPL-2.0-or-later.

use std::collections::HashMap;
use std::sync::OnceLock;

use qa_core::identity::{same_actor, ActorId};
use qa_core::math::{Bounds, Vec3};

use crate::q1::addons::context::{addon_emit, set_addon_number, set_addon_vector, set_combat_team, Q1AddonEvent};
use crate::q1::addons::monsters::ai::{register_mg3_monster_source, Mg3ActionHandler, Mg3Monster, Mg3SourceHooks};
use crate::q1::addons::monsters::startup::mg3_use_mapped;
use crate::q1::base::animation::MonsterAi;
use crate::q1::base::creatures::{monster_controller, store_monster_controller};
use crate::q1::base::monsters::BaseMonsterState;
use crate::q1::base::projectiles::throw_gib;
use crate::q1::base::species::{MonsterMovement, MonsterSpecies};
use crate::q1::foundation::callbacks::{Q1ActionHandler, Q1CallbackHandlers, Q1TouchHandler};
use crate::q1::foundation::entity::{Q1MonsterSpecies, Q1ProjectileKind};
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::gameplay::BodyPatch;
use crate::q1::foundation::types::{
    normalize, vadd, vscale, vsub, Q1Event, Q1MoveType, Q1Solid, Q1SoundChannel, POINT, ZERO,
};
use crate::q1::Q1Error;

use super::frames::szombie::frames;
use super::registry::register_boss_controllers;

/// Shub zombie callback prefix.
const SZOMBIE_PREFIX: &str = "mg3:szombie";

/// Shub zombie spawn defaults (`spec`).
const SZOMBIE_SPEC: MonsterSpecies = MonsterSpecies {
    species: Q1MonsterSpecies::Zombie,
    kill_string: None,
    classnames: &["monster_szombie"],
    model: "zombie",
    head: None,
    health: 60.0,
    gib_health: f64::NEG_INFINITY,
    gibs: &[],
    bounds: Bounds {
        min: Vec3 {
            x: -16.0,
            y: -16.0,
            z: -24.0,
        },
        max: Vec3 {
            x: 16.0,
            y: 16.0,
            z: 32.0,
        },
    },
    stand: "szombie_stand1",
    walk: "szombie_walk1",
    run: "szombie_run1",
    sight: "",
    missile: None,
    melee: true,
    movement: MonsterMovement::Walk,
};

/// Throws a zombie meat grenade (`grenade`).
fn szombie_grenade(monster: &mut Mg3Monster, offset: Vec3) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    monster
        .monster
        .game
        .sound(&id, "zombie/z_shot1.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
    let missile = monster.monster.game.create("szombie_grenade", None, None)?;
    monster.monster.game.update_entity(&missile, |entity| {
        entity.classname = String::new();
        entity.owner = Some(id.clone());
        entity.movement = Q1MoveType::Bounce;
        entity.solid = Q1Solid::Bbox;
    })?;
    let basis = monster.monster.game.basis;
    let origin = monster.monster.origin()?;
    let muzzle = vadd(
        vadd(
            vadd(origin, vscale(basis.forward, f64::from(offset.x))),
            vscale(basis.right, f64::from(offset.y)),
        ),
        vscale(basis.up, f64::from(offset.z - 24.0)),
    );
    let angles = monster.monster.game.body(&id)?.angles;
    monster.monster.game.make_vectors(angles);
    let target = monster.monster.target()?.unwrap_or(ZERO);
    let velocity = vscale(normalize(vsub(target, muzzle)), 600.0);
    monster.monster.game.set_body(
        &missile,
        &BodyPatch {
            origin: Some(muzzle),
            velocity: Some(Vec3 {
                x: velocity.x,
                y: velocity.y,
                z: 200.0,
            }),
            bounds: Some(POINT),
            ..Default::default()
        },
    )?;
    let touch = monster
        .monster
        .game
        .named
        .touch(&format!("{SZOMBIE_PREFIX}:grenade_touch"))?;
    monster.monster.game.update_entity(&missile, |entity| {
        entity.angular_velocity = Vec3 {
            x: 3000.0,
            y: 1000.0,
            z: 2000.0,
        };
        entity.model = String::from("progs/zom_gib.mdl");
        entity.touch = Some(touch);
    })?;
    monster.monster.game.schedule(&missile, 2.5, "SUB_Remove")?;
    monster.monster.game.link(&missile)
}

/// Shub zombie frame actions (`actions`).
fn szombie_actions() -> &'static HashMap<String, Mg3ActionHandler> {
    static ACTIONS: OnceLock<HashMap<String, Mg3ActionHandler>> = OnceLock::new();
    ACTIONS.get_or_init(|| {
        fn stand1(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.ai(MonsterAi::Stand, 0.0)
        }
        fn walk1(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.ai(MonsterAi::Walk, 0.0)
        }
        fn walk2(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.ai(MonsterAi::Walk, 2.0)
        }
        fn walk3(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.ai(MonsterAi::Walk, 3.0)
        }
        fn walk5(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.ai(MonsterAi::Walk, 1.0)
        }
        fn walk19(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.ai(MonsterAi::Walk, 0.0)?;
            let id = monster.monster.id.clone();
            if monster.monster.game.host.random() < 0.2 {
                monster
                    .monster
                    .game
                    .sound(&id, "zombie/z_idle.wav", Q1SoundChannel::Voice, 2.0, 1.0)?;
            }
            Ok(())
        }
        fn run1(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.ai(MonsterAi::Run, 1.0)?;
            monster.monster.controller.in_pain = 0.0;
            Ok(())
        }
        fn run2(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.ai(MonsterAi::Run, 1.0)
        }
        fn run3(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.ai(MonsterAi::Run, 0.0)
        }
        fn run5(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.ai(MonsterAi::Run, 2.0)
        }
        fn run6(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.ai(MonsterAi::Run, 3.0)
        }
        fn run7(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.ai(MonsterAi::Run, 4.0)
        }
        fn run15(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.ai(MonsterAi::Run, 6.0)
        }
        fn run16(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.ai(MonsterAi::Run, 7.0)
        }
        fn run18(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.ai(MonsterAi::Run, 8.0)?;
            let id = monster.monster.id.clone();
            if monster.monster.game.host.random() < 0.2 {
                monster
                    .monster
                    .game
                    .sound(&id, "zombie/z_idle.wav", Q1SoundChannel::Voice, 2.0, 1.0)?;
            }
            if monster.monster.game.host.random() > 0.8 {
                monster
                    .monster
                    .game
                    .sound(&id, "zombie/z_idle1.wav", Q1SoundChannel::Voice, 2.0, 1.0)?;
            }
            Ok(())
        }
        fn atta1(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.face()
        }
        fn atta13(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.face()?;
            szombie_grenade(
                monster,
                Vec3 {
                    x: -10.0,
                    y: -22.0,
                    z: 30.0,
                },
            )
        }
        fn attb14(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.face()?;
            szombie_grenade(
                monster,
                Vec3 {
                    x: -10.0,
                    y: -24.0,
                    z: 29.0,
                },
            )
        }
        fn attc12(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.monster.face()?;
            szombie_grenade(
                monster,
                Vec3 {
                    x: -12.0,
                    y: -19.0,
                    z: 29.0,
                },
            )
        }
        fn paina1(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            monster
                .monster
                .game
                .sound(&id, "zombie/z_pain.wav", Q1SoundChannel::Voice, 1.0, 1.0)
        }
        fn painb1(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            monster
                .monster
                .game
                .sound(&id, "zombie/z_pain1.wav", Q1SoundChannel::Voice, 1.0, 1.0)
        }
        fn paina2(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.ai(MonsterAi::Painforward, 3.0)
        }
        fn paina3(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.ai(MonsterAi::Painforward, 1.0)
        }
        fn paina4(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.ai(MonsterAi::Pain, 1.0)
        }
        fn paina5(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.ai(MonsterAi::Pain, 3.0)
        }
        fn painb2(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.ai(MonsterAi::Pain, 2.0)
        }
        fn painb3(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.ai(MonsterAi::Pain, 8.0)
        }
        fn painb4(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.ai(MonsterAi::Pain, 6.0)
        }
        fn painb9(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            monster
                .monster
                .game
                .sound(&id, "zombie/z_fall.wav", Q1SoundChannel::Body, 1.0, 1.0)
        }
        fn paine1(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            monster
                .monster
                .game
                .sound(&id, "zombie/z_pain.wav", Q1SoundChannel::Voice, 1.0, 1.0)?;
            let owned = monster.monster.game.entity_ref(&id).map(|entity| entity.actor.clone());
            if let Some(owned) = owned {
                monster.monster.game.host.combat.set_health(&owned, 60.0)?;
            }
            Ok(())
        }
        fn paine3(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.ai(MonsterAi::Pain, 5.0)
        }
        fn paine10(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            monster
                .monster
                .game
                .sound(&id, "zombie/z_fall.wav", Q1SoundChannel::Body, 1.0, 1.0)?;
            monster
                .monster
                .game
                .update_entity(&id, |entity| entity.solid = Q1Solid::None)?;
            monster.monster.game.link(&id)
        }
        fn paine11(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            let next_think = monster
                .monster
                .game
                .entity_ref(&id)
                .map(|entity| entity.next_think)
                .unwrap_or(0.0);
            let time = monster.monster.game.time;
            monster.monster.delay(next_think - time + 5.0)?;
            let owned = monster.monster.game.entity_ref(&id).map(|entity| entity.actor.clone());
            if let Some(owned) = owned {
                monster.monster.game.host.combat.set_health(&owned, 60.0)?;
            }
            Ok(())
        }
        fn paine12(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            let owned = monster.monster.game.entity_ref(&id).map(|entity| entity.actor.clone());
            if let Some(owned) = owned {
                monster.monster.game.host.combat.set_health(&owned, 60.0)?;
            }
            monster
                .monster
                .game
                .sound(&id, "zombie/z_idle.wav", Q1SoundChannel::Voice, 2.0, 1.0)?;
            monster
                .monster
                .game
                .update_entity(&id, |entity| entity.solid = Q1Solid::Slidebox)?;
            let owned = monster.monster.game.entity_ref(&id).map(|entity| entity.actor.clone());
            if let Some(owned) = owned {
                if !monster.monster.game.host.walk_move(&owned, 0.0, 0.0) {
                    monster.monster.controller.next_frame = String::from("szombie_paine11");
                    monster
                        .monster
                        .game
                        .update_entity(&id, |entity| entity.solid = Q1Solid::None)?;
                }
            }
            monster.monster.game.link(&id)
        }
        fn paine25(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            monster.ai(MonsterAi::Painforward, 5.0)
        }
        HashMap::from([
            (String::from("szombie:szombie_stand1"), stand1 as Mg3ActionHandler),
            (String::from("szombie:szombie_walk1"), walk1 as Mg3ActionHandler),
            (String::from("szombie:szombie_walk2"), walk2 as Mg3ActionHandler),
            (String::from("szombie:szombie_walk3"), walk3 as Mg3ActionHandler),
            (String::from("szombie:szombie_walk5"), walk5 as Mg3ActionHandler),
            (String::from("szombie:szombie_walk19"), walk19 as Mg3ActionHandler),
            (String::from("szombie:szombie_run1"), run1 as Mg3ActionHandler),
            (String::from("szombie:szombie_run2"), run2 as Mg3ActionHandler),
            (String::from("szombie:szombie_run3"), run3 as Mg3ActionHandler),
            (String::from("szombie:szombie_run5"), run5 as Mg3ActionHandler),
            (String::from("szombie:szombie_run6"), run6 as Mg3ActionHandler),
            (String::from("szombie:szombie_run7"), run7 as Mg3ActionHandler),
            (String::from("szombie:szombie_run15"), run15 as Mg3ActionHandler),
            (String::from("szombie:szombie_run16"), run16 as Mg3ActionHandler),
            (String::from("szombie:szombie_run18"), run18 as Mg3ActionHandler),
            (String::from("szombie:szombie_atta1"), atta1 as Mg3ActionHandler),
            (String::from("szombie:szombie_atta13"), atta13 as Mg3ActionHandler),
            (String::from("szombie:szombie_attb14"), attb14 as Mg3ActionHandler),
            (String::from("szombie:szombie_attc12"), attc12 as Mg3ActionHandler),
            (String::from("szombie:szombie_paina1"), paina1 as Mg3ActionHandler),
            (String::from("szombie:szombie_painb1"), painb1 as Mg3ActionHandler),
            (String::from("szombie:szombie_paina2"), paina2 as Mg3ActionHandler),
            (String::from("szombie:szombie_paina3"), paina3 as Mg3ActionHandler),
            (String::from("szombie:szombie_paina4"), paina4 as Mg3ActionHandler),
            (String::from("szombie:szombie_paina5"), paina5 as Mg3ActionHandler),
            (String::from("szombie:szombie_painb2"), painb2 as Mg3ActionHandler),
            (String::from("szombie:szombie_painb3"), painb3 as Mg3ActionHandler),
            (String::from("szombie:szombie_painb4"), painb4 as Mg3ActionHandler),
            (String::from("szombie:szombie_painb9"), painb9 as Mg3ActionHandler),
            (String::from("szombie:szombie_paine1"), paine1 as Mg3ActionHandler),
            (String::from("szombie:szombie_paine3"), paine3 as Mg3ActionHandler),
            (String::from("szombie:szombie_paine10"), paine10 as Mg3ActionHandler),
            (String::from("szombie:szombie_paine11"), paine11 as Mg3ActionHandler),
            (String::from("szombie:szombie_paine12"), paine12 as Mg3ActionHandler),
            (String::from("szombie:szombie_paine25"), paine25 as Mg3ActionHandler),
        ])
    })
}

fn szombie_load_controller(
    game: &Q1EntityServices,
    id: &ActorId,
    _classname: &str,
) -> Result<(BaseMonsterState, &'static MonsterSpecies), Q1Error> {
    let controller = monster_controller(game, id, "monster_szombie")?;
    Ok((controller, &SZOMBIE_SPEC))
}

fn szombie_play(monster: &mut Mg3Monster, name: &str) -> Result<(), Q1Error> {
    match name.strip_prefix("zombie_") {
        Some(rest) => monster.play_default(&format!("szombie_{rest}")),
        None => monster.play_default(name),
    }
}

fn szombie_melee(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    let rolled = monster.monster.game.host.random();
    monster.play(if rolled < 0.3 {
        "szombie_atta1"
    } else if rolled < 0.6 {
        "szombie_attb1"
    } else {
        "szombie_attc1"
    })
}

fn szombie_die(monster: &mut Mg3Monster, attacker: Option<&ActorId>) -> Result<(), Q1Error> {
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
    monster
        .monster
        .game
        .sound(&id, "zombie/z_gib.wav", Q1SoundChannel::Voice, 1.0, 1.0)?;
    let origin = monster.monster.origin()?;
    let health = monster.monster.game.health(&id);
    for model in ["gib1", "gib2", "gib3"] {
        throw_gib(monster.monster.game, origin, model, health)?;
    }
    monster.monster.game.remove(&id)
}

fn spawn_szombie(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let mut monster = Mg3Monster::spawn_new(game, SZOMBIE_PREFIX, id, &SZOMBIE_SPEC)?;
    let owned = monster
        .monster
        .game
        .entity_ref(&monster.monster.id.clone())
        .map(|entity| entity.actor.clone());
    if let Some(owned) = owned {
        monster.monster.game.host.combat.set_health(&owned, 60.0)?;
    }
    monster.monster.controller.in_pain = 2.0;
    let id = monster.monster.id.clone();
    let owner_origin = monster
        .monster
        .game
        .entity_ref(&id)
        .and_then(|entity| entity.owner.clone())
        .and_then(|owner| monster.monster.game.body(&owner).ok())
        .map(|body| body.origin);
    monster.monster.game.update_entity(&id, |entity| {
        entity.classname = String::from("monster_szombie");
        entity.max_health = 60.0;
        entity.solid = Q1Solid::None;
        entity.movement = Q1MoveType::Step;
        entity.model = String::from("progs/zombie.mdl");
    })?;
    monster.monster.game.set_bounds(&id, SZOMBIE_SPEC.bounds)?;
    monster.monster.game.set_origin(&id, owner_origin.unwrap_or(ZERO))?;
    let angles = monster.monster.game.body(&id)?.angles;
    monster.monster.game.update_entity(&id, |entity| {
        entity.ideal_yaw = f64::from(angles.y);
        if entity.yaw_speed == 0.0 {
            entity.yaw_speed = 20.0;
        }
        entity.movement_flags |= 32;
    })?;
    monster.monster.game.set_damageable(&id, true)?;
    let owned = monster.monster.game.entity_ref(&id).map(|entity| entity.actor.clone());
    if let Some(owned) = owned {
        set_combat_team(monster.monster.game, &owned, Some("q1:monsters"))?;
    }
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
            z: 25.0,
        },
    )?;
    set_addon_number(monster.monster.game, &id, "combat_style", 2.0)?;
    let pain = monster
        .monster
        .game
        .named
        .pain(&format!("{SZOMBIE_PREFIX}:monster_pain"))?;
    let die = monster
        .monster
        .game
        .named
        .die(&format!("{SZOMBIE_PREFIX}:monster_die"))?;
    monster.monster.game.update_entity(&id, |entity| {
        entity.pain = Some(pain);
        entity.die = Some(die);
    })?;
    monster.monster.game.total_monsters += 1;
    let total = monster.monster.game.total_monsters;
    monster.monster.game.host.emit(Q1Event::MonsterTotal { total });
    let enemy = (monster.monster.game.host.players)().into_iter().next();
    monster.monster.monster.enemy = enemy;
    monster.monster.game.update_entity(&id, |entity| entity.frame = 174)?;
    monster.monster.controller.current_frame = String::from("szombie_paine10");
    monster.monster.controller.next_frame = String::from("szombie_paine11");
    monster.monster.delay(2.0)?;
    monster.finish()
}

/// Spawns a Shub zombie at an eligible spawn point (`spawnShubZombie`).
pub fn spawn_shub_zombie(game: &mut Q1EntityServices) -> Result<Option<ActorId>, Q1Error> {
    let ids = game.entity_ids();
    let zombies = ids
        .iter()
        .filter(|id| {
            game.entity_ref(id)
                .is_some_and(|entity| entity.classname == "monster_szombie")
        })
        .count();
    if zombies > 32 {
        return Ok(None);
    }
    let spawns: Vec<ActorId> = ids
        .into_iter()
        .filter(|id| {
            game.entity_ref(id)
                .is_some_and(|entity| entity.classname == "info_szombie_spawn")
        })
        .collect();
    let eligible = spawns
        .iter()
        .filter(|id| {
            game.entity_ref(id)
                .is_some_and(|entity| entity.wait != 0.0 && entity.wait < game.time)
        })
        .count();
    if eligible == 0 {
        return Ok(None);
    }
    let index = (game.host.random() * (eligible as f64 - 1.0) + 0.5).floor() as usize;
    let point = spawns
        .get(index)
        .cloned()
        .ok_or_else(|| crate::q1::q1_error("Missing source zombie spawn"))?;
    let time = game.time;
    game.update_entity(&point, |entity| entity.wait = time + 8.0)?;
    let zombie = game.create("monster_szombie", None, None)?;
    let owner = game.entity_ref(&point).map(|entity| entity.actor.id().clone());
    game.update_entity(&zombie, |entity| entity.owner = owner)?;
    game.spawn_entity(&zombie, None)?;
    Ok(Some(zombie))
}

/// Spawns a homing flame from a source (`spawnHomingFlame`).
pub fn spawn_homing_flame(game: &mut Q1EntityServices, source: &ActorId) -> Result<ActorId, Q1Error> {
    let flame = game.create("homing_flame", None, None)?;
    game.update_entity(&flame, |entity| {
        entity.classname = String::new();
        entity.solid = Q1Solid::Bbox;
        entity.movement = Q1MoveType::Flymissile;
        entity.model = String::from("progs/flame2.mdl");
        entity.effects = 64;
    })?;
    let stored = game
        .entity_ref(source)
        .and_then(|entity| entity.monster.clone())
        .and_then(|monster| monster.enemy)
        .or_else(|| {
            game.entity_ref(source)
                .and_then(|entity| entity.references.get("enemy").cloned().flatten())
        });
    let enemy = match stored {
        Some(enemy) if game.is_player(&enemy) => Some(enemy),
        _ => (game.host.players)().into_iter().next(),
    };
    game.update_entity(&flame, |entity| {
        entity.references.insert(String::from("enemy"), enemy.clone());
        entity.speed = 400.0;
    })?;
    let origin = vadd(game.body(source)?.origin, Vec3 { x: 0.0, y: 0.0, z: 4.0 });
    let target_origin = enemy
        .as_ref()
        .and_then(|enemy| game.host.bodies.read(enemy))
        .map(|body| body.origin)
        .unwrap_or(ZERO);
    game.set_body(
        &flame,
        &BodyPatch {
            origin: Some(origin),
            velocity: Some(vscale(normalize(vsub(target_origin, origin)), 400.0)),
            bounds: Some(POINT),
            ..Default::default()
        },
    )?;
    let touch = game.named.touch("projectile_touch")?;
    game.update_entity(&flame, |entity| {
        entity.projectile = Some(Q1ProjectileKind::Spike);
        entity.touch = Some(touch);
    })?;
    let time = game.time;
    set_addon_number(game, &flame, "waitmin", time)?;
    game.schedule(&flame, 0.1, &format!("{SZOMBIE_PREFIX}:homing_flame_think"))?;
    game.link(&flame)?;
    game.remove(source)?;
    Ok(flame)
}

fn szombie_grenade_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    let owner = game.entity_ref(id).and_then(|entity| entity.owner.clone());
    if owner.as_ref().is_some_and(|owner| same_actor(owner, other)) {
        return Ok(());
    }
    let classname = game.host.classname(other);
    if classname == "monster_oldone_new" || classname == "oldnew_child" {
        return game.remove(id);
    }
    if game
        .host
        .combat
        .read(other)
        .is_some_and(|combat| combat.can_take_damage)
    {
        let attacker = game.entity_ref(id).and_then(|entity| entity.owner.clone());
        game.damage(other, Some(id), attacker.as_ref(), 10.0, &Q1DamageParams::default());
        game.sound(id, "zombie/z_hit.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
        return game.remove(id);
    }
    game.sound(id, "zombie/z_miss.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
    game.set_body(
        id,
        &BodyPatch {
            velocity: Some(ZERO),
            ..Default::default()
        },
    )?;
    game.update_entity(id, |entity| entity.angular_velocity = ZERO)?;
    // The donor throws here too: `SUB_Remove` registers an action, not a
    // touch (`named.touch` rejects it).
    let remove = game.named.touch("SUB_Remove")?;
    game.update_entity(id, |entity| entity.touch = Some(remove))
}

fn homing_flame_think(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let origin = game.body(id)?.origin;
    let elapsed = game
        .entity_ref(id)
        .map(|entity| entity.number("waitmin"))
        .unwrap_or(0.0)
        + 3.0
        < game.time;
    if elapsed {
        addon_emit(
            game,
            Q1AddonEvent::ColoredExplosion {
                origin,
                color_start: 244,
                color_length: 3,
            },
        )?;
        let world = game.world.clone();
        game.radius_damage(id, Some(id), 100.0, world.as_ref(), None, "");
        return game.remove(id);
    }
    game.update_entity(id, |entity| {
        entity.speed = (entity.speed - 10.0).max(0.0);
    })?;
    let speed = game.entity_ref(id).map(|entity| entity.speed).unwrap_or(0.0);
    let enemy = game
        .entity_ref(id)
        .and_then(|entity| entity.references.get("enemy").cloned().flatten());
    let enemy_origin = enemy
        .as_ref()
        .and_then(|enemy| game.host.bodies.read(enemy))
        .map(|body| body.origin)
        .unwrap_or(ZERO);
    let direction = normalize(vsub(enemy_origin, origin));
    let velocity = game.body(id)?.velocity;
    game.set_body(
        id,
        &BodyPatch {
            velocity: Some(vscale(
                vadd(vscale(direction, 0.3), vscale(normalize(velocity), 0.7)),
                speed,
            )),
            ..Default::default()
        },
    )?;
    game.schedule(id, 0.1, &format!("{SZOMBIE_PREFIX}:homing_flame_think"))
}

fn spawn_info_szombie_spawn(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| entity.wait = -1.0)
}

/// Registers Shub zombies (`registerShubZombie`).
pub fn register_shub_zombie(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    register_mg3_monster_source(
        SZOMBIE_PREFIX,
        frames(),
        szombie_actions(),
        Mg3SourceHooks {
            melee_attack: Some(szombie_melee),
            die: Some(szombie_die),
            use_monster: Some(mg3_use_mapped),
            play: Some(szombie_play),
            ..Default::default()
        },
        szombie_load_controller,
        store_monster_controller,
    );
    register_boss_controllers(game, SZOMBIE_PREFIX, "monster_szombie", spawn_szombie)?;
    game.register_spawn("info_szombie_spawn", spawn_info_szombie_spawn)?;
    game.named.register(
        &format!("{SZOMBIE_PREFIX}:grenade_touch"),
        Q1CallbackHandlers {
            touch: Some(szombie_grenade_touch as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{SZOMBIE_PREFIX}:homing_flame_think"),
        Q1CallbackHandlers {
            action: Some(homing_flame_think as Q1ActionHandler),
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
        register_shub_zombie(game).expect("szombie");
        guard
    }

    #[test]
    fn spawn_sets_szombie_defaults() {
        let mut game = test_game();
        let _guard = setup(&mut game);
        let _player = attach_test_player(&mut game);
        let zombie = game.create("monster_szombie", None, None).expect("zombie");
        game.spawn_entity(&zombie, None).expect("spawn");
        let entity = game.entity_ref(&zombie).expect("entity");
        assert_eq!(entity.max_health, 60.0);
        assert_eq!(entity.model, "progs/zombie.mdl");
        assert_eq!(entity.frame, 174);
        assert_eq!(entity.number("combat_style"), 2.0);
        assert_eq!(game.total_monsters, 1);
    }

    #[test]
    fn shub_zombie_spawns_at_eligible_points() {
        let mut game = test_game();
        let _guard = setup(&mut game);
        assert_eq!(spawn_shub_zombie(&mut game).expect("none"), None);
        let point = game.create("info_szombie_spawn", None, None).expect("point");
        game.spawn_entity(&point, None).expect("spawn");
        let spawned = spawn_shub_zombie(&mut game).expect("zombie");
        assert!(spawned.is_some());
        assert_eq!(game.total_monsters, 1);
        assert_eq!(spawn_shub_zombie(&mut game).expect("cooldown"), None);
    }

    #[test]
    fn homing_flame_tracks_and_consumes_source() {
        let mut game = test_game();
        let _guard = setup(&mut game);
        let player = attach_test_player(&mut game);
        let source = game.create("monster_szombie", None, None).expect("source");
        game.spawn_entity(&source, None).expect("spawn");
        game.update_entity(&source, |entity| {
            entity.references.insert(String::from("enemy"), Some(player.clone()));
        })
        .expect("enemy");
        let flame = spawn_homing_flame(&mut game, &source).expect("flame");
        assert!(game.entity_ref(&source).is_none());
        let entity = game.entity_ref(&flame).expect("flame");
        assert_eq!(entity.model, "progs/flame2.mdl");
        assert_eq!(entity.references.get("enemy").cloned().flatten(), Some(player));
    }
}
