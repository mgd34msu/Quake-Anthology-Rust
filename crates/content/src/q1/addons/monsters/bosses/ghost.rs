//! Q1 mg3 player ghost (`src/content/q1/addons/monsters/bosses/ghost.ts`).
//!
//! `quakec_mg3/monsters/mg3_player_ghost.qc`. GPL-2.0-or-later.

use std::collections::HashMap;
use std::sync::OnceLock;

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

use crate::q1::addons::context::{removed_for_runes, removed_outside_coop};
use crate::q1::addons::monsters::ai::{register_mg3_monster_source, Mg3ActionHandler, Mg3Monster, Mg3SourceHooks};
use crate::q1::addons::monsters::startup::mg3_use_mapped;
use crate::q1::base::creatures::{monster_controller, store_monster_controller};
use crate::q1::base::map_entities::spawn_bubble;
use crate::q1::base::monsters::BaseMonsterState;
use crate::q1::base::species::{MonsterMovement, MonsterSpecies};
use crate::q1::foundation::callbacks::{Q1ActionHandler, Q1CallbackHandlers, Q1TouchHandler};
use crate::q1::foundation::entity::Q1MonsterSpecies;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::BodyPatch;
use crate::q1::foundation::spawns::spawn_teleport_fog;
use crate::q1::foundation::types::{normalize, vadd, vscale, vsub, Q1MoveType, Q1Solid, Q1SoundChannel, ZERO};
use crate::q1::missionpacks::types::velocity_angles;
use crate::q1::Q1Error;

use super::frames::ghost::frames;
use super::registry::register_boss_controllers;

/// Ghost callback prefix.
const GHOST_PREFIX: &str = "mg3:ghost";

/// Ghost spawn defaults (`spec`).
const GHOST_SPEC: MonsterSpecies = MonsterSpecies {
    species: Q1MonsterSpecies::Army,
    kill_string: None,
    classnames: &["monster_ghost"],
    model: "player",
    head: None,
    health: 10.0,
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
    stand: "ghost_stand1",
    walk: "ghost_run1",
    run: "ghost_run1",
    sight: "",
    missile: None,
    melee: false,
    movement: MonsterMovement::Fly,
};

/// Ghost frame actions (`actions`).
fn ghost_actions() -> &'static HashMap<String, Mg3ActionHandler> {
    static ACTIONS: OnceLock<HashMap<String, Mg3ActionHandler>> = OnceLock::new();
    ACTIONS.get_or_init(|| {
        fn stand1(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            monster.monster.game.set_body(
                &id,
                &BodyPatch {
                    velocity: Some(ZERO),
                    ..Default::default()
                },
            )
        }
        fn stand5(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            let wait = monster
                .monster
                .game
                .entity_ref(&id)
                .map(|entity| entity.wait)
                .unwrap_or(0.0);
            let yaw = monster.monster.game.host.random() as f32 * 360.0;
            let forward = monster
                .monster
                .game
                .make_vectors(Vec3 { x: 0.0, y: yaw, z: 0.0 })
                .forward;
            let mut finished = false;
            monster.monster.game.update_entity(&id, |entity| {
                entity.count -= 1.0;
                if entity.count == 0.0 {
                    finished = true;
                    entity.dest2 = vadd(entity.dest1, vscale(forward, wait));
                }
            })?;
            if finished {
                monster.monster.controller.next_frame = String::from("ghost_run1");
            }
            Ok(())
        }
        fn run1(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            let origin = monster.monster.origin()?;
            let dest2 = monster
                .monster
                .game
                .entity_ref(&id)
                .map(|entity| entity.dest2)
                .unwrap_or(ZERO);
            let delta = vsub(dest2, origin);
            let direction = normalize(Vec3 {
                x: delta.x,
                y: delta.y,
                z: 0.0,
            });
            monster.monster.game.set_body(
                &id,
                &BodyPatch {
                    angles: Some(velocity_angles(direction)),
                    velocity: Some(vscale(direction, 150.0)),
                    ..Default::default()
                },
            )
        }
        fn run6(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            let count = (monster.monster.game.host.random() * 5.0 + 0.5).floor() + 3.0;
            monster.monster.game.update_entity(&id, |entity| entity.count = count)
        }
        fn diea1(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            let water = monster
                .monster
                .game
                .entity_ref(&id)
                .map(|entity| entity.water_level)
                .unwrap_or(0);
            if water == 3 {
                let origin = monster.monster.origin()?;
                let timer = monster.monster.game.create("death_bubbles", None, None)?;
                monster.monster.game.update_entity(&timer, |entity| {
                    entity.owner = Some(id.clone());
                    entity.count = 20.0;
                })?;
                monster.monster.game.set_origin(&timer, origin)?;
                monster
                    .monster
                    .game
                    .schedule(&timer, 0.1, &format!("{GHOST_PREFIX}:death_bubbles"))?;
                monster
                    .monster
                    .game
                    .sound(&id, "player/h2odeath.wav", Q1SoundChannel::Voice, 0.0, 1.0)?;
            } else {
                let index = (monster.monster.game.host.random() * 4.0 + 1.5).floor() as i32;
                let path = format!("player/death{index}.wav");
                monster.monster.game.update_entity(&id, |entity| {
                    entity.fields.insert(String::from("noise"), path.clone());
                })?;
                monster
                    .monster
                    .game
                    .sound(&id, &path, Q1SoundChannel::Voice, 0.0, 1.0)?;
            }
            monster.monster.game.set_body(
                &id,
                &BodyPatch {
                    velocity: Some(ZERO),
                    ..Default::default()
                },
            )
        }
        fn diea11(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
            let id = monster.monster.id.clone();
            let origin = monster.monster.origin()?;
            spawn_teleport_fog(monster.monster.game, origin)?;
            monster.monster.game.remove(&id)
        }
        HashMap::from([
            (String::from("ghost:ghost_stand1"), stand1 as Mg3ActionHandler),
            (String::from("ghost:ghost_stand5"), stand5 as Mg3ActionHandler),
            (String::from("ghost:ghost_run1"), run1 as Mg3ActionHandler),
            (String::from("ghost:ghost_run6"), run6 as Mg3ActionHandler),
            (String::from("ghost:ghost_diea1"), diea1 as Mg3ActionHandler),
            (String::from("ghost:ghost_diea11"), diea11 as Mg3ActionHandler),
        ])
    })
}

fn ghost_load_controller(
    game: &Q1EntityServices,
    id: &ActorId,
    _classname: &str,
) -> Result<(BaseMonsterState, &'static MonsterSpecies), Q1Error> {
    let controller = monster_controller(game, id, "monster_ghost")?;
    Ok((controller, &GHOST_SPEC))
}

fn ghost_die(monster: &mut Mg3Monster, _attacker: Option<&ActorId>) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    let death = monster
        .monster
        .game
        .entity_ref(&id)
        .map(|entity| entity.text("ghost.death"))
        .unwrap_or_default();
    monster.play(&death)
}

fn ghost_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    let mut monster = Mg3Monster::load(game, id)?;
    monster.monster.game.host.random();
    let id = monster.monster.id.clone();
    if monster.monster.game.health(other) == 0.0
        || monster.monster.monster.pain_finished > monster.monster.game.time
        || !monster.monster.game.is_player(other)
    {
        return monster.finish();
    }
    monster.monster.game.update_entity(&id, |entity| entity.touch = None)?;
    ghost_die(&mut monster, Some(other))?;
    monster.finish()
}

fn death_bubbles(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let owner = game.entity_ref(id).and_then(|entity| entity.owner.clone());
    let Some(owner) = owner else {
        return Ok(());
    };
    let owner_state = game
        .entity_ref(&owner)
        .map(|entity| (entity.water_level, game.body(&owner).map(|body| body.origin)));
    let Some((water_level, Ok(origin))) = owner_state else {
        return Ok(());
    };
    if water_level != 3 {
        return Ok(());
    }
    spawn_bubble(
        game,
        vadd(
            origin,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 24.0,
            },
        ),
        Vec3 {
            x: 0.0,
            y: 0.0,
            z: 15.0,
        },
        false,
    )?;
    let mut done = false;
    game.update_entity(id, |entity| {
        entity.count -= 1.0;
        done = entity.count <= 0.0;
    })?;
    if done {
        game.remove(id)
    } else {
        game.schedule(id, 0.1, &format!("{GHOST_PREFIX}:death_bubbles"))
    }
}

fn spawn_ghost(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let mut monster = Mg3Monster::spawn_new(game, GHOST_PREFIX, id, &GHOST_SPEC)?;
    let removed = removed_for_runes(monster.monster.game, &monster.monster.id.clone())?
        || removed_outside_coop(monster.monster.game, &monster.monster.id.clone(), true)?;
    if removed {
        return monster.finish();
    }
    let choice = monster.monster.game.host.random() * 5.0;
    let death = if choice < 1.0 {
        "ghost_diea1"
    } else if choice < 2.0 {
        "ghost_dieb1"
    } else if choice < 3.0 {
        "ghost_diec1"
    } else if choice < 4.0 {
        "ghost_died1"
    } else {
        "ghost_diee1"
    };
    let id = monster.monster.id.clone();
    monster.monster.game.update_entity(&id, |entity| {
        entity.fields.insert(String::from("ghost.death"), String::from(death));
        if entity.wait == 0.0 {
            entity.wait = 128.0;
        }
    })?;
    let owned = monster.monster.game.entity_ref(&id).map(|entity| entity.actor.clone());
    if let Some(owned) = owned {
        monster.monster.game.host.combat.set_health(&owned, 10.0)?;
    }
    let path_end = monster
        .monster
        .game
        .named
        .action(&format!("{GHOST_PREFIX}:monster_stand"))?;
    let touch = monster.monster.game.named.touch(&format!("{GHOST_PREFIX}:touch"))?;
    let die = monster.monster.game.named.die(&format!("{GHOST_PREFIX}:monster_die"))?;
    monster.monster.game.update_entity(&id, |entity| {
        entity.max_health = 10.0;
        entity.solid = Q1Solid::Slidebox;
        entity.model = String::from("progs/player.mdl");
        entity.aimed_damage = false;
        entity.path_end = Some(path_end);
        entity.touch = Some(touch);
        entity.die = Some(die);
        entity.movement = Q1MoveType::Fly;
    })?;
    monster.monster.game.set_damageable(&id, true)?;
    monster.monster.game.set_bounds(&id, GHOST_SPEC.bounds)?;
    let origin = monster.monster.origin()?;
    let yaw = monster.monster.game.host.random() as f32 * 360.0;
    let forward = monster
        .monster
        .game
        .make_vectors(Vec3 { x: 0.0, y: yaw, z: 0.0 })
        .forward;
    let wait = monster
        .monster
        .game
        .entity_ref(&id)
        .map(|entity| entity.wait)
        .unwrap_or(0.0);
    monster.monster.game.update_entity(&id, |entity| {
        entity.dest1 = origin;
        entity.dest2 = vadd(origin, vscale(forward, wait));
    })?;
    monster.play(GHOST_SPEC.stand)?;
    let count = (monster.monster.game.host.random() * 5.0 + 0.5).floor() + 3.0;
    monster.monster.game.update_entity(&id, |entity| entity.count = count)?;
    monster.finish()
}

/// Registers player ghosts (`registerGhost`).
pub fn register_ghost(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    register_mg3_monster_source(
        GHOST_PREFIX,
        frames(),
        ghost_actions(),
        Mg3SourceHooks {
            use_monster: Some(mg3_use_mapped),
            die: Some(ghost_die),
            ..Default::default()
        },
        ghost_load_controller,
        store_monster_controller,
    );
    register_boss_controllers(game, GHOST_PREFIX, "monster_ghost", spawn_ghost)?;
    game.named.register(
        &format!("{GHOST_PREFIX}:touch"),
        Q1CallbackHandlers {
            touch: Some(ghost_touch as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{GHOST_PREFIX}:death_bubbles"),
        Q1CallbackHandlers {
            action: Some(death_bubbles as Q1ActionHandler),
            ..Default::default()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::{register_test_addons, Q1AddonProgram};
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::missionpacks::types::test_game;

    fn setup(game: &mut Q1EntityServices) -> Q1BaseGuard {
        let guard = Q1BaseGuard::register(game, Q1BaseOptions::default()).expect("base");
        register_test_addons(game, Q1AddonProgram::Mg3);
        register_ghost(game).expect("ghost");
        guard
    }

    #[test]
    fn spawn_sets_ghost_defaults() {
        let mut game = test_game();
        let _guard = setup(&mut game);
        let ghost = game.create("monster_ghost", None, None).expect("ghost");
        game.spawn_entity(&ghost, None).expect("spawn");
        let entity = game.entity_ref(&ghost).expect("entity");
        assert_eq!(entity.model, "progs/player.mdl");
        assert_eq!(entity.max_health, 10.0);
        assert_eq!(entity.movement, Q1MoveType::Fly);
        assert!(entity.text("ghost.death").starts_with("ghost_die"));
        assert_eq!(entity.wait, 128.0);
        assert!(entity.count >= 3.0 && entity.count <= 8.0);
    }

    #[test]
    fn run_action_steers_toward_dest() {
        let mut game = test_game();
        let _guard = setup(&mut game);
        let ghost = game.create("monster_ghost", None, None).expect("ghost");
        game.spawn_entity(&ghost, None).expect("spawn");
        game.set_body(
            &ghost,
            &BodyPatch {
                origin: Some(ZERO),
                ..Default::default()
            },
        )
        .expect("origin");
        game.update_entity(&ghost, |entity| {
            entity.dest2 = Vec3 {
                x: 100.0,
                y: 0.0,
                z: 0.0,
            };
        })
        .expect("dest");
        let mut monster = Mg3Monster::load(&mut game, &ghost).expect("load");
        ghost_actions()["ghost:ghost_run1"](&mut monster).expect("run");
        monster.finish().expect("finish");
        let body = game.body(&ghost).expect("body");
        assert_eq!(body.velocity.x, 150.0);
        assert_eq!(body.velocity.y, 0.0);
    }
}
