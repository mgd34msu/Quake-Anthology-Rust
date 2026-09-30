//! Hipnotic grunt and rottweiler definitions
//! (`src/content/q1/missionpacks/monsters/charmed-base.ts`).

use std::rc::Rc;

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

use crate::q1::base::animation::MonsterAi;
use crate::q1::base::projectiles::{drop_backpack, throw_gib, throw_head, BackpackDrop};
use crate::q1::base::species::{MonsterMovement, MonsterSpecies};
use crate::q1::foundation::entity::{Q1AttackState, Q1MonsterSpecies};
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::gameplay::TouchSurface;
use crate::q1::foundation::types::{
    length, normalize, vadd, vscale, vsub, Q1SoundChannel, Q1TraceRequest, POINT,
};
use crate::q1::foundation::weapons::fire_bullets;
use crate::q1::missionpacks::types::Q1MissionPack;
use crate::q1::Q1Error;

use super::helpers::HUMAN_BOUNDS;
use super::runtime::{mission_pack_monsters, MissionMonster, Q1MissionPackMonsters};
use super::tables::grunt::FRAMES as ARMY_FRAMES;
use super::tables::rottweiler::FRAMES as DOG_FRAMES;
use super::types::{leaked_name, MissionAction, PackMonsterDefinition};

fn army_fire(monster: &mut MissionMonster) {
    monster.face();
    let id = monster.entity.actor.id.clone();
    let _ = monster
        .game
        .sound(&id, "soldier/sattck1.wav", Q1SoundChannel::Weapon, 1.0, 1.0);
    let Some(enemy) = monster.enemy.clone() else {
        monster.refresh();
        return;
    };
    let Some(enemy_body) = monster.game.host.bodies.read(&enemy) else {
        monster.refresh();
        return;
    };
    let direction = normalize(vsub(
        vsub(enemy_body.origin, vscale(enemy_body.velocity, 0.2)),
        monster.origin,
    ));
    let view_angles = monster.entity.vector("v_angle");
    let owned = monster.entity.actor.clone();
    fire_bullets(
        monster.game,
        &owned,
        direction,
        view_angles,
        4,
        0.1,
        0.1,
        None,
    );
    monster.refresh();
}

fn drop_shells(monster: &mut MissionMonster) {
    monster.sync();
    monster.entity.solid = crate::q1::foundation::types::Q1Solid::None;
    monster.flush_entity();
    let id = monster.entity.actor.id.clone();
    let _ = monster.game.link(&id);
    let _ = drop_backpack(
        monster.game,
        monster.origin,
        &BackpackDrop {
            shells: 5.0,
            ..Default::default()
        },
        None,
    );
    monster.refresh();
}

/// Hipnotic grunt definition (`hipnoticArmyDefinition`).
pub fn hipnotic_army_definition() -> PackMonsterDefinition {
    let mut actions: Vec<(&'static str, MissionAction)> = vec![
        ("army_fire", Rc::new(army_fire)),
        (
            "grunt:army_atk5",
            Rc::new(|monster: &mut MissionMonster| {
                monster.face();
                army_fire(monster);
                monster.entity.effects |= 2;
                monster.refresh();
            }),
        ),
        (
            "grunt:army_atk7",
            Rc::new(|monster: &mut MissionMonster| {
                monster.face();
                if monster.game.options().skill == 3
                    && !monster.state.refired
                    && monster.visible(None)
                {
                    monster.state.refired = true;
                    monster.next_frame = "army_atk1".to_string();
                }
                monster.refresh();
            }),
        ),
        ("grunt:army_die3", Rc::new(drop_shells)),
        (
            "grunt:army_cdie3",
            Rc::new(|monster: &mut MissionMonster| {
                drop_shells(monster);
                let id = monster.entity.actor.id.clone();
                let yaw = monster
                    .game
                    .body(&id)
                    .map(|body| f64::from(body.angles.y))
                    .unwrap_or(0.0);
                let owned = monster.entity.actor.clone();
                monster.game.host.walk_move(&owned, yaw + 180.0, 4.0);
                monster.refresh();
            }),
        ),
    ];
    for distance in [3.0, 4.0, 5.0, 13.0] {
        actions.push((
            leaked_name(format!("ai_back({distance})")),
            Rc::new(move |monster: &mut MissionMonster| {
                let id = monster.entity.actor.id.clone();
                let yaw = monster
                    .game
                    .body(&id)
                    .map(|body| f64::from(body.angles.y))
                    .unwrap_or(0.0);
                let owned = monster.entity.actor.clone();
                monster.game.host.walk_move(&owned, yaw + 180.0, distance);
                monster.refresh();
            }),
        ));
    }
    PackMonsterDefinition {
        spec: Box::leak(Box::new(MonsterSpecies {
            species: Q1MonsterSpecies::Army,
            kill_string: None,
            classnames: &["monster_army"],
            model: "soldier",
            head: Some("h_guard"),
            health: 30.0,
            gib_health: -35.0,
            gibs: &["gib1", "gib2", "gib3"],
            bounds: HUMAN_BOUNDS,
            stand: "army_stand1",
            walk: "army_walk1",
            run: "army_run1",
            sight: "soldier/sight1.wav",
            missile: Some("army_atk1"),
            melee: false,
            movement: MonsterMovement::Walk,
        })),
        base_behavior: false,
        frames: ARMY_FRAMES,
        actions,
        callbacks: Vec::new(),
        spawn: None,
        start: None,
        pain: Rc::new(|monster: &mut MissionMonster, _, _| {
            if monster.state.pain_finished > monster.game.time {
                monster.refresh();
                return;
            }
            let roll = monster.game.host.random();
            monster.state.pain_finished = monster.game.time + if roll < 0.2 { 0.6 } else { 1.1 };
            let frame = if roll < 0.2 {
                "army_pain1"
            } else if roll < 0.6 {
                "army_painb1"
            } else {
                "army_painc1"
            };
            monster.play(frame);
            let id = monster.entity.actor.id.clone();
            let _ = monster.game.sound_simple(
                &id,
                if roll < 0.2 {
                    "soldier/pain1.wav"
                } else {
                    "soldier/pain2.wav"
                },
            );
            monster.refresh();
        }),
        die: Rc::new(|monster: &mut MissionMonster, _| {
            let id = monster.entity.actor.id.clone();
            let health = monster.game.health(&id);
            if health < -35.0 {
                let _ = monster.game.sound_simple(&id, "player/udeath.wav");
                let _ = throw_head(monster.game, &id, "h_guard", health);
                for model in ["gib1", "gib2", "gib3"] {
                    let _ = throw_gib(monster.game, monster.origin, model, health);
                }
                monster.refresh();
                return;
            }
            let _ = monster.game.sound_simple(&id, "soldier/death1.wav");
            monster.play(if monster.game.host.random() < 0.5 {
                "army_die1"
            } else {
                "army_cdie1"
            });
            monster.refresh();
        }),
        melee: None,
        check_attack: Some(Rc::new(|monster: &mut MissionMonster| {
            let start = monster.eye(None);
            let end = monster
                .enemy
                .clone()
                .and_then(|enemy| monster.eye(Some(&enemy)));
            let (Some(start), Some(end)) = (start, end) else {
                monster.refresh();
                return false;
            };
            let id = monster.entity.actor.id.clone();
            let trace = monster.game.host.trace(&Q1TraceRequest {
                start,
                end,
                bounds: POINT,
                ignore: Some(id),
                monsters: true,
                missile: false,
            });
            let range = monster.range_distance(None);
            if trace.in_open && trace.in_water
                || trace.actor != monster.enemy
                || monster.game.time < monster.state.attack_finished
                || range >= 1000.0
            {
                monster.refresh();
                return false;
            }
            let chance = if range < 120.0 {
                0.9
            } else if range < 500.0 {
                0.4
            } else {
                0.05
            };
            if monster.game.host.random() >= chance {
                monster.refresh();
                return false;
            }
            monster.play("army_atk1");
            monster.attack_finished(1.0 + monster.game.host.random());
            if monster.game.host.random() < 0.3 {
                monster.lefty = !monster.lefty;
            }
            monster.refresh();
            true
        })),
        found: None,
        ai: None,
        use_: None,
    }
}

fn dog_jump_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let id = id.clone();
    let other = other.clone();
    let mut local: Q1MissionPackMonsters = mission_pack_monsters(&Q1MissionPack::Hipnotic)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let Ok(mut monster) = local.require(game, &id) else {
        return Ok(());
    };
    if game.health(&id) <= 0.0 {
        return Ok(());
    }
    let damageable = game
        .host
        .combat
        .read(&other)
        .is_some_and(|combat| combat.can_take_damage);
    let speed = game
        .body(&id)
        .map(|body| length(body.velocity))
        .unwrap_or(0.0);
    if damageable && speed > 300.0 {
        game.damage(
            &other,
            Some(&id),
            Some(&id),
            10.0 + 10.0 * game.host.random(),
            &Q1DamageParams::default(),
        );
    }
    if !game.host.check_bottom(&id) {
        if monster.entity.movement_flags & 512 != 0 {
            monster.entity.touch = None;
            monster.next_frame = "dog_leap1".to_string();
            monster.flush_entity();
            monster.delay(0.1);
        }
        monster.finish();
        *mission_pack_monsters(&Q1MissionPack::Hipnotic)
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = local;
        return Ok(());
    }
    monster.entity.touch = None;
    monster.next_frame = "dog_run1".to_string();
    monster.flush_entity();
    monster.delay(0.1);
    monster.finish();
    *mission_pack_monsters(&Q1MissionPack::Hipnotic)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = local;
    Ok(())
}

/// Hipnotic rottweiler definition (`hipnoticDogDefinition`).
pub fn hipnotic_dog_definition(runtime: &Q1MissionPackMonsters) -> PackMonsterDefinition {
    debug_assert!(
        matches!(runtime.pack, Q1MissionPack::Hipnotic),
        "dog is Hipnotic-only"
    );
    PackMonsterDefinition {
        spec: Box::leak(Box::new(MonsterSpecies {
            species: Q1MonsterSpecies::Dog,
            kill_string: None,
            classnames: &["monster_dog"],
            model: "dog",
            head: Some("h_dog"),
            health: 25.0,
            gib_health: -35.0,
            gibs: &["gib3", "gib3", "gib3"],
            bounds: Bounds {
                min: Vec3 {
                    x: -32.0,
                    y: -32.0,
                    z: -24.0,
                },
                max: Vec3 {
                    x: 32.0,
                    y: 32.0,
                    z: 40.0,
                },
            },
            stand: "dog_stand1",
            walk: "dog_walk1",
            run: "dog_run1",
            sight: "dog/dsight.wav",
            missile: Some("dog_leap1"),
            melee: true,
            movement: MonsterMovement::Walk,
        })),
        base_behavior: false,
        frames: DOG_FRAMES,
        actions: vec![
            (
                "dog_bite",
                Rc::new(|monster: &mut MissionMonster| {
                    let Some(enemy) = monster.enemy.clone() else {
                        monster.refresh();
                        return;
                    };
                    monster.ai(MonsterAi::Charge, 10.0);
                    let id = monster.entity.actor.id.clone();
                    if !monster.game.can_damage(&enemy, &id) || monster.distance() > 100.0 {
                        monster.refresh();
                        return;
                    }
                    let rolls = monster.game.host.random()
                        + monster.game.host.random()
                        + monster.game.host.random();
                    monster.game.damage(
                        &enemy,
                        Some(&id),
                        Some(&id),
                        rolls * 8.0,
                        &Q1DamageParams::default(),
                    );
                    monster.refresh();
                }),
            ),
            (
                "rottweiler:dog_leap2",
                Rc::new(|monster: &mut MissionMonster| {
                    monster.face();
                    let id = monster.entity.actor.id.clone();
                    if let Ok(touch) = monster.game.named.touch("hipnotic:Dog_JumpTouch") {
                        monster.entity.touch = Some(touch);
                        monster.flush_entity();
                    }
                    let angles = monster
                        .game
                        .body(&id)
                        .map(|body| body.angles)
                        .unwrap_or(Vec3 {
                            x: 0.0,
                            y: 0.0,
                            z: 0.0,
                        });
                    let basis = monster.game.make_vectors(angles);
                    let origin = vadd(
                        monster.origin,
                        Vec3 {
                            x: 0.0,
                            y: 0.0,
                            z: 1.0,
                        },
                    );
                    let velocity = vadd(
                        vscale(basis.forward, 300.0),
                        Vec3 {
                            x: 0.0,
                            y: 0.0,
                            z: 200.0,
                        },
                    );
                    let _ = monster.game.set_body(
                        &id,
                        &crate::q1::foundation::gameplay::BodyPatch {
                            origin: Some(origin),
                            velocity: Some(velocity),
                            ..Default::default()
                        },
                    );
                    monster.entity.movement_flags &= !512;
                    monster.refresh();
                }),
            ),
        ],
        callbacks: vec![(
            "Dog_JumpTouch",
            crate::q1::foundation::callbacks::Q1CallbackHandlers {
                touch: Some(dog_jump_touch),
                ..Default::default()
            },
        )],
        spawn: None,
        start: None,
        pain: Rc::new(|monster: &mut MissionMonster, _, _| {
            let id = monster.entity.actor.id.clone();
            let _ = monster.game.sound_simple(&id, "dog/dpain1.wav");
            monster.play(if monster.game.host.random() > 0.5 {
                "dog_pain1"
            } else {
                "dog_painb1"
            });
            monster.refresh();
        }),
        die: Rc::new(|monster: &mut MissionMonster, _| {
            let id = monster.entity.actor.id.clone();
            let health = monster.game.health(&id);
            if health < -35.0 {
                let _ = monster.game.sound_simple(&id, "player/udeath.wav");
                for _ in 0..3 {
                    let _ = throw_gib(monster.game, monster.origin, "gib3", health);
                }
                let _ = throw_head(monster.game, &id, "h_dog", health);
                monster.refresh();
                return;
            }
            let _ = monster.game.sound_simple(&id, "dog/ddeath.wav");
            monster.entity.solid = crate::q1::foundation::types::Q1Solid::None;
            monster.flush_entity();
            let _ = monster.game.link(&id);
            monster.play(if monster.game.host.random() > 0.5 {
                "dog_die1"
            } else {
                "dog_dieb1"
            });
            monster.refresh();
        }),
        melee: Some(Rc::new(|monster: &mut MissionMonster| {
            monster.play("dog_atta1");
            monster.refresh();
        })),
        check_attack: Some(Rc::new(|monster: &mut MissionMonster| {
            if monster.range_distance(None) < 120.0 {
                monster.entity.attack_state = Q1AttackState::Melee;
                monster.refresh();
                return true;
            }
            let id = monster.entity.actor.id.clone();
            let body = monster.game.body(&id);
            let enemy_body = monster
                .enemy
                .clone()
                .and_then(|enemy| monster.game.host.bodies.read(&enemy));
            let (Ok(body), Some(enemy_body)) = (body, enemy_body) else {
                monster.refresh();
                return false;
            };
            let feet = body.origin.z + body.bounds.min.z;
            let enemy_feet = enemy_body.origin.z + enemy_body.bounds.min.z;
            let enemy_height = enemy_body.bounds.max.z - enemy_body.bounds.min.z;
            if feet > enemy_feet + 0.75 * enemy_height
                || body.origin.z + body.bounds.max.z < enemy_feet + 0.25 * enemy_height
            {
                monster.refresh();
                return false;
            }
            let distance =
                (enemy_body.origin.x - body.origin.x).hypot(enemy_body.origin.y - body.origin.y);
            if distance < 80.0 || distance > 150.0 {
                monster.refresh();
                return false;
            }
            monster.entity.attack_state = Q1AttackState::Missile;
            monster.refresh();
            true
        })),
        found: None,
        ai: None,
        use_: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::missionpacks::types::test_game;

    use super::super::types::MissionMonsterHooks;

    #[test]
    fn army_definition_shape() {
        let definition = hipnotic_army_definition();
        assert_eq!(definition.spec.classnames, ["monster_army"]);
        assert!(definition
            .actions
            .iter()
            .any(|(name, _)| *name == "army_fire"));
        assert!(definition
            .actions
            .iter()
            .any(|(name, _)| *name == "ai_back(3)"));
        assert!(definition.check_attack.is_some());
    }

    #[test]
    fn dog_definition_shape() {
        let mut game = test_game();
        let runtime = Q1MissionPackMonsters::new(
            &mut game,
            Q1MissionPack::Hipnotic,
            MissionMonsterHooks::default(),
        )
        .expect("new");
        let definition = hipnotic_dog_definition(&runtime);
        assert_eq!(definition.spec.classnames, ["monster_dog"]);
        assert!(definition
            .callbacks
            .iter()
            .any(|(name, _)| *name == "Dog_JumpTouch"));
        assert!(definition.melee.is_some());
    }

    #[test]
    fn pack_attacks_rest_without_enemy() {
        let mut game = test_game();
        let mut runtime = Q1MissionPackMonsters::new(
            &mut game,
            Q1MissionPack::Hipnotic,
            MissionMonsterHooks::default(),
        )
        .expect("new");
        runtime
            .register(&mut game, hipnotic_army_definition())
            .expect("army");
        runtime
            .register(&mut game, hipnotic_dog_definition(&runtime.clone()))
            .expect("dog");
        let army = game.create("monster_army", None, None).expect("army");
        let mut monster = runtime.require(&mut game, &army).expect("require");
        assert!(!monster.try_attack());
        monster.finish();
        let dog = game.create("monster_dog", None, None).expect("dog");
        let mut monster = runtime.require(&mut game, &dog).expect("require");
        assert!(!monster.try_attack());
        monster.finish();
    }
}
