//! Player-model decoys (`src/content/q1/missionpacks/monsters/decoy.ts`).

use std::rc::Rc;

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::q1::base::species::{MonsterMovement, MonsterSpecies};
use crate::q1::foundation::entity::Q1MonsterSpecies;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::types::{
    vsub, yaw_for, Q1Event, Q1MoveType, Q1Solid, Q1SoundChannel, ZERO,
};
use crate::q1::missionpacks::types::Q1MissionPack;
use crate::q1::Q1Error;

use super::helpers::{number, HULL_BOUNDS};
use super::runtime::{mission_pack_monsters, MissionMonster, Q1MissionPackMonsters};
use super::tables::hipdecoy::FRAMES;
use super::types::PackMonsterDefinition;

fn setup(monster: &mut MissionMonster) {
    monster.sync();
    let id = monster.entity.actor.id.clone();
    monster.entity.model = "progs/player.mdl".to_string();
    let _ = monster.game.set_bounds(&id, HULL_BOUNDS);
    monster
        .entity
        .fields
        .insert("view_ofs".to_string(), "0 0 22".to_string());
    monster.entity.solid = Q1Solid::Slidebox;
    monster.entity.movement = Q1MoveType::Step;
    monster.entity.max_health = 3000000.0;
    let owned = monster.entity.actor.clone();
    let _ = monster.game.host.combat.set_health(&owned, 3000000.0);
    let player = monster.game.host.players().first().cloned();
    let colormap = player
        .as_ref()
        .and_then(|player| monster.game.entity(player))
        .map(|entity| entity.number("colormap"))
        .unwrap_or(0.0);
    monster
        .entity
        .fields
        .insert("colormap".to_string(), (colormap as f32).to_string());
    monster.refresh();
}

/// Decoy definition (`decoyDefinition`).
pub fn decoy_definition() -> PackMonsterDefinition {
    PackMonsterDefinition {
        spec: Box::leak(Box::new(MonsterSpecies {
            species: Q1MonsterSpecies::Decoy,
            kill_string: None,
            classnames: &["monster_decoy"],
            model: "player",
            head: None,
            health: 3000000.0,
            gib_health: f64::NEG_INFINITY,
            gibs: &[],
            bounds: HULL_BOUNDS,
            stand: "decoy_stand1",
            walk: "decoy_walk1",
            run: "decoy_walk1",
            sight: "",
            missile: Some("decoy_stand1"),
            melee: false,
            movement: MonsterMovement::Walk,
        })),
        base_behavior: false,
        frames: FRAMES,
        actions: vec![
            (
                "hipdecoy:decoy_stand1",
                Rc::new(|monster: &mut MissionMonster| {
                    monster.change_yaw();
                    let mut walk = monster.entity.number("walkframe");
                    if walk >= 5.0 {
                        walk = 0.0;
                    }
                    monster.entity.frame = 12 + walk as i32;
                    number(monster, "walkframe", walk + 1.0);
                    if monster.game.time > monster.state.pause_until {
                        monster.play("decoy_walk1");
                    }
                    monster.refresh();
                }),
            ),
            (
                "hipdecoy:decoy_walk1",
                Rc::new(|monster: &mut MissionMonster| {
                    let goal = monster
                        .entity
                        .references
                        .get("goalentity")
                        .cloned()
                        .flatten()
                        .and_then(|goal| {
                            monster
                                .game
                                .entity(&goal)
                                .map(|entity| entity.actor.id.clone())
                        })
                        .or_else(|| monster.game.find(&monster.state.path).first().cloned());
                    if let Some(goal) = goal {
                        let owned = monster.entity.actor.clone();
                        monster.game.host.move_to_goal(&owned, &goal, 12.0, None);
                    }
                    number(monster, "weaponframe", 0.0);
                    let mut walk = monster.entity.number("walkframe");
                    if walk == 6.0 {
                        walk = 0.0;
                    }
                    if walk == 2.0 || walk == 5.0 {
                        let roll = monster.game.host.random();
                        let step = if roll < 0.14 {
                            1
                        } else if roll < 0.29 {
                            2
                        } else if roll < 0.43 {
                            3
                        } else if roll < 0.58 {
                            4
                        } else if roll < 0.72 {
                            5
                        } else if roll < 0.86 {
                            6
                        } else {
                            7
                        };
                        let id = monster.entity.actor.id.clone();
                        monster.game.host.emit(Q1Event::Sound {
                            origin: None,
                            actor: id,
                            path: format!("misc/foot{step}.wav"),
                            channel: Q1SoundChannel::Voice,
                            attenuation: 1.0,
                            volume: 0.5,
                        });
                    }
                    monster.entity.frame += walk as i32;
                    number(monster, "walkframe", walk + 1.0);
                    monster.refresh();
                }),
            ),
        ],
        callbacks: Vec::new(),
        spawn: Some(Rc::new(|monster: &mut MissionMonster| {
            setup(monster);
            monster.spawn_default();
            monster.game.total_monsters -= 1;
            monster.refresh();
        })),
        start: None,
        pain: Rc::new(|monster, _, _| {
            monster.play("decoy_stand1");
        }),
        die: Rc::new(|monster, _| {
            monster.play("decoy_stand1");
        }),
        melee: None,
        check_attack: None,
        found: None,
        ai: None,
        use_: None,
    }
}

/// Spawn a decoy at an origin (`becomeDecoy`).
pub fn become_decoy(
    game: &mut Q1EntityServices,
    target: &str,
    origin: Vec3,
) -> Result<ActorId, Q1Error> {
    let mut local: Q1MissionPackMonsters = mission_pack_monsters(&Q1MissionPack::Hipnotic)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let id = game.create("monster_decoy", None, None)?;
    let mut monster = local.adopt(game, &id)?;
    if let Some(world) = monster.game.world.clone() {
        let decoy = monster.entity.actor.id.clone();
        let _ = monster.game.update_entity(&world, |entity| {
            entity
                .references
                .insert("hipdecoy".to_string(), Some(decoy));
        });
    }
    setup(&mut monster);
    let _ = monster.game.set_origin(&id, origin);
    monster.entity.target = target.to_string();
    monster.state.path = target.to_string();
    monster.entity.aimed_damage = true;
    let yaw = monster
        .game
        .body(&id)
        .map(|body| f64::from(body.angles.y))
        .unwrap_or(0.0);
    monster.entity.ideal_yaw = yaw;
    if monster.entity.yaw_speed == 0.0 {
        monster.entity.yaw_speed = 20.0;
    }
    let use_callback = monster.game.named.use_callback("hipnotic:monster_use")?;
    monster.entity.use_callback = Some(use_callback);
    monster.entity.movement_flags |= 32;
    monster.flush_entity();
    monster.game.set_damageable(&id, true)?;
    monster.refresh();
    let destination = monster.game.find(target).first().cloned();
    if !target.is_empty() {
        let destination_entity = destination
            .as_ref()
            .and_then(|destination| monster.game.entity(destination).cloned());
        monster
            .entity
            .references
            .insert("goalentity".to_string(), destination.clone());
        monster
            .entity
            .references
            .insert("movetarget".to_string(), destination.clone());
        let goal_origin = destination
            .as_ref()
            .and_then(|destination| monster.game.body(destination).ok())
            .map(|body| body.origin)
            .or_else(|| {
                monster
                    .game
                    .world
                    .clone()
                    .and_then(|world| monster.game.body(&world).ok())
                    .map(|body| body.origin)
            })
            .unwrap_or(ZERO);
        monster.entity.ideal_yaw = yaw_for(vsub(goal_origin, origin));
        monster.flush_entity();
        if destination_entity.map(|entity| entity.classname.as_str()) == Some("path_corner") {
            monster.play("decoy_walk1");
        } else {
            monster.state.pause_until = 99999999.0;
        }
        monster.play("decoy_stand1");
    } else {
        monster.state.pause_until = 99999999.0;
        monster.play("decoy_stand1");
    }
    let think_in = monster.entity.next_think - monster.game.time + monster.game.host.random() * 0.5;
    monster.delay(think_in);
    monster.finish();
    *mission_pack_monsters(&Q1MissionPack::Hipnotic)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = local;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::missionpacks::types::test_game;

    use super::super::types::MissionMonsterHooks;

    #[test]
    fn decoy_definition_shape() {
        let definition = decoy_definition();
        assert_eq!(definition.spec.classnames, ["monster_decoy"]);
        assert_eq!(definition.spec.health, 3000000.0);
        assert_eq!(definition.actions.len(), 2);
        assert!(definition.spawn.is_some());
    }

    #[test]
    fn become_decoy_links() {
        let mut game = test_game();
        let mut runtime = Q1MissionPackMonsters::new(
            &mut game,
            Q1MonsterPack::Hipnotic,
            MissionMonsterHooks::default(),
        )
        .expect("new");
        runtime
            .register(&mut game, decoy_definition())
            .expect("register");
        let id = become_decoy(&mut game, "", ZERO).expect("decoy");
        let entity = game.entity(&id).expect("entity").clone();
        assert_eq!(entity.classname, "monster_decoy");
        assert_eq!(entity.model, "progs/player.mdl");
        assert_eq!(entity.max_health, 3000000.0);
    }
}
