//! Gremlin corpse search and evasive goals (`src/content/q1/missionpacks/monsters/gremlin-ai.ts`).

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::q1::base::animation::MonsterAi;
use crate::q1::foundation::entity::Q1AttackState;
use crate::q1::foundation::types::{length, normalize, vadd, vscale, vsub, yaw_for, Q1TraceRequest, POINT};
use crate::q1::missionpacks::types::velocity_angles;

use super::gremlin_weapons::gremlin_has_ammo;
use super::helpers::{number, radius_actors};
use super::runtime::MissionMonster;

/// Pick a live victim, preferring players (`gremlinFindVictim`).
pub fn gremlin_find_victim(monster: &mut MissionMonster) -> Option<ActorId> {
    let time = monster.game.time;
    monster.state.search_until = time + 1.0;
    let id = monster.entity.actor.id().clone();
    let mut selected = None;
    let mut distance = 1000.0;
    for actor in radius_actors(monster.game, monster.origin, 1000.0) {
        let other = monster.game.entity(&actor).cloned();
        let flags = other.as_ref().map(|entity| entity.movement_flags).unwrap_or(0)
            | if monster.game.is_player(&actor) { 8 } else { 0 };
        let body = monster.game.host.bodies.read(&actor);
        if flags & 128 != 0
            || flags & (32 | 8) == 0
            || !monster.visible(Some(&actor))
            || monster.game.health(&actor) <= 0.0
            || actor == id
            || body.is_none()
        {
            continue;
        }
        let body = body.expect("body");
        let mut range = f64::from(length(vsub(body.origin, monster.origin)));
        if Some(&actor)
            == monster
                .entity
                .references
                .get("lastvictim")
                .and_then(|reference| reference.as_ref())
        {
            range *= 2.0;
        }
        if flags & 8 != 0 {
            range /= 1.5;
        }
        if other.as_ref().map(|entity| entity.classname.as_str()) == Some(monster.entity.classname.as_str()) {
            range *= 1.5;
        }
        if range < distance {
            distance = range;
            selected.clone_from(&Some(actor));
        }
    }
    monster
        .entity
        .references
        .insert("lastvictim".to_string(), selected.clone());
    monster.flush_entity();
    selected
}

/// Search for corpses or victims before the shared scan (`gremlinFindTarget`).
pub fn gremlin_find_target(monster: &mut MissionMonster) -> bool {
    monster.sync();
    if monster.entity.number("stoleweapon") == 0.0 && monster.game.time > monster.entity.wait {
        let time = monster.game.time;
        monster.entity.wait = time + 1.0;
        monster.flush_entity();
        let mut distance = 2000.0;
        let mut gorge = None;
        for actor in monster.game.host.actors.observations() {
            let other = monster.game.entity(&actor.id).cloned();
            let body = monster.game.host.bodies.read(&actor.id);
            let flags = other.as_ref().map(|entity| entity.movement_flags).unwrap_or(0)
                | if monster.game.is_player(&actor.id) { 8 } else { 0 };
            let Some(body) = body else { continue };
            if monster.game.health(&actor.id) >= 1.0 || flags & (32 | 8) == 0 {
                continue;
            }
            let vertical = (f64::from(body.origin.z) - f64::from(monster.origin.z)).abs();
            let visible = monster.visible(Some(&actor.id));
            let (start, end) = (monster.eye(None), monster.eye(Some(&actor.id)));
            let range = match (start, end) {
                (Some(start), Some(end)) => f64::from(length(vsub(end, start))),
                _ => f64::INFINITY,
            };
            if visible
                && vertical < 80.0
                && other.as_ref().map(|entity| entity.number("gorging")).unwrap_or(0.0) == 0.0
                && range < distance
            {
                distance = range;
                gorge = Some(actor.id.clone());
            }
        }
        if let Some(gorge) = gorge.filter(|_| distance < 700.0 * monster.game.host.random()) {
            monster.state.old_enemy.clone_from(&monster.enemy);
            number(monster, "gorging", 1.0);
            monster.enemy = Some(gorge.clone());
            let time = monster.game.time;
            monster.state.search_until = time + 4.0;
            monster.found(&gorge);
            monster.refresh();
            return true;
        }
    } else if monster.entity.number("stoleweapon") != 0.0 {
        if let Some(victim) = gremlin_find_victim(monster) {
            monster.found(&victim);
            let time = monster.game.time;
            monster.state.attack_finished = time;
            monster.state.search_until = time + 2.0;
            monster.refresh();
            return true;
        }
    }
    let found = monster.find_target();
    let time = monster.game.time;
    monster.state.search_until = time + 2.0;
    monster.refresh();
    found
}

/// Patrol toward the path goal (`gremlinWalk`).
pub fn gremlin_walk(monster: &mut MissionMonster, distance: f64) {
    if gremlin_find_target(monster) {
        return;
    }
    monster.sync();
    let goal = monster
        .entity
        .references
        .get("goalentity")
        .cloned()
        .flatten()
        .or_else(|| monster.game.find(&monster.state.path.clone()).first().cloned());
    if let Some(goal) = goal {
        let owned = monster.entity.actor.clone();
        monster.game.host.move_to_goal(&owned, &goal, distance, None);
    }
    monster.refresh();
}

/// Idle until the patrol pause expires (`gremlinStand`).
pub fn gremlin_stand(monster: &mut MissionMonster) {
    if monster.find_target() {
        return;
    }
    if monster.game.time > monster.state.pause_until {
        monster.play("gremlin_walk1");
    }
}

/// Chase corpses or strafe away from victims (`gremlinRun`).
pub fn gremlin_run(monster: &mut MissionMonster, distance: f64) {
    monster.sync();
    let id = monster.entity.actor.id().clone();
    if monster.entity.water_type == -5 {
        let world = monster.game.world.clone();
        let inflictor = world.clone().unwrap_or_else(|| id.clone());
        monster.game.damage(
            &id,
            Some(&inflictor),
            world.as_ref(),
            2000.0,
            &crate::q1::foundation::entity_services::Q1DamageParams::default(),
        );
    }
    if monster.entity.number("stoleweapon") != 0.0 {
        monster.entity.frame += 164 - 29;
        monster.flush_entity();
    }
    let Some(target) = monster.target else {
        monster.ai(MonsterAi::Run, distance);
        return;
    };
    if monster.entity.number("gorging") != 0.0 {
        let blocked = monster
            .game
            .host
            .trace(&Q1TraceRequest {
                start: monster.origin,
                end: target,
                bounds: POINT,
                ignore: Some(id.clone()),
                monsters: false,
                missile: false,
            })
            .fraction
            != 1.0
            || !monster.visible(None);
        if blocked {
            number(monster, "gorging", 0.0);
            return;
        }
        let range = monster.distance();
        if range < 130.0 {
            monster.face();
            if range < 45.0 {
                monster.melee_attack();
                monster.entity.attack_state = Q1AttackState::Straight;
                monster.flush_entity();
                return;
            }
            let owned = monster.entity.actor.clone();
            let yaw = monster
                .game
                .body(&id)
                .map(|body| f64::from(body.angles.y))
                .unwrap_or(0.0);
            if !monster.game.host.walk_move(&owned, yaw, distance) {
                number(monster, "gorging", 0.0);
            }
            return;
        }
        if let Some(enemy) = monster.enemy.clone() {
            let owned = monster.entity.actor.clone();
            monster.game.host.move_to_goal(&owned, &enemy, distance, None);
        }
        monster.refresh();
        return;
    }
    if monster.game.host.random() > 0.97 && gremlin_find_target(monster) {
        return;
    }
    if monster.entity.number("stoleweapon") != 0.0 {
        if let Some(enemy) = monster.enemy.clone() {
            if monster.game.health(&enemy) < 0.0 && monster.game.host.classname(&enemy) == "player" {
                monster.play("gremlin_glook1");
                return;
            }
        }
        let mut goal = monster.entity.references.get("trigger_field").cloned().flatten();
        if !gremlin_has_ammo(monster) {
            if monster.entity.number("t_length") == 1.0 {
                if let Some(goal) = goal.take() {
                    let _ = monster.game.remove(&goal);
                }
                let enemy = monster.enemy.clone();
                monster.entity.references.insert("goalentity".to_string(), enemy);
                monster.flush_entity();
                number(monster, "t_length", 0.0);
            }
            return;
        }
        let range = monster.distance();
        let direction = normalize(vsub(monster.origin, target));
        if monster.entity.number("t_length") == 0.0 && range < 150.0 {
            let created = monster.game.create("gremlin_goal", None, None).expect("gremlin goal");
            let _ = monster.game.set_bounds(
                &created,
                qa_core::math::Bounds {
                    min: Vec3 {
                        x: -1.0,
                        y: -1.0,
                        z: -1.0,
                    },
                    max: Vec3 { x: 1.0, y: 1.0, z: 1.0 },
                },
            );
            monster
                .entity
                .references
                .insert("trigger_field".to_string(), Some(created.clone()));
            monster.flush_entity();
            number(monster, "t_length", 1.0);
            goal = Some(created);
        }
        if monster.entity.number("t_length") == 1.0 {
            if let Some(goal) = goal.clone() {
                if range > 250.0 {
                    let _ = monster.game.remove(&goal);
                    let enemy = monster.enemy.clone();
                    monster.entity.references.insert("goalentity".to_string(), enemy);
                    monster.flush_entity();
                    number(monster, "t_length", 0.0);
                } else {
                    if range < 160.0 {
                        let mut angles = velocity_angles(direction);
                        let mut end = target;
                        for _ in 0..10 {
                            end = vadd(target, vscale(monster.game.make_vectors(angles).forward, 350.0));
                            let clear = monster
                                .game
                                .host
                                .trace(&Q1TraceRequest {
                                    start: target,
                                    end,
                                    bounds: POINT,
                                    ignore: Some(id.clone()),
                                    monsters: true,
                                    missile: false,
                                })
                                .fraction
                                == 1.0
                                && monster
                                    .game
                                    .host
                                    .trace(&Q1TraceRequest {
                                        start: monster.origin,
                                        end,
                                        bounds: POINT,
                                        ignore: Some(id.clone()),
                                        monsters: true,
                                        missile: false,
                                    })
                                    .fraction
                                    == 1.0;
                            angles = Vec3 {
                                x: angles.x,
                                y: (f64::from(angles.y) + 36.0) as f32 % 360.0,
                                z: angles.z,
                            };
                            if clear {
                                break;
                            }
                        }
                        let _ = monster.game.set_origin(&goal, end);
                    }
                    monster
                        .entity
                        .references
                        .insert("goalentity".to_string(), Some(goal.clone()));
                    monster.flush_entity();
                    if let Ok(goal_body) = monster.game.body(&goal) {
                        monster.entity.ideal_yaw = yaw_for(normalize(vsub(goal_body.origin, monster.origin)));
                        monster.flush_entity();
                    }
                    monster.change_yaw();
                    let owned = monster.entity.actor.clone();
                    monster.game.host.move_to_goal(&owned, &goal, distance, None);
                    monster.delay(0.1);
                    return;
                }
            }
        }
    }
    monster.ai(MonsterAi::Run, distance);
    monster.delay(0.1);
}

#[cfg(test)]
mod tests {
    use super::{gremlin_find_target, gremlin_run, gremlin_stand, gremlin_walk};
    use crate::q1::missionpacks::monsters::gremlin::gremlin_definition;
    use crate::q1::missionpacks::monsters::runtime::Q1MissionPackMonsters;
    use crate::q1::missionpacks::monsters::types::MissionMonsterHooks;
    use crate::q1::missionpacks::types::{test_game, Q1MissionPack};

    #[test]
    fn gremlin_ai_helpers_settle() {
        let mut game = test_game();
        let mut runtime =
            Q1MissionPackMonsters::new(&mut game, Q1MissionPack::Hipnotic, MissionMonsterHooks::default())
                .expect("runtime");
        runtime
            .register(&mut game, gremlin_definition(&runtime))
            .expect("register");
        let id = game.create("monster_gremlin", None, None).expect("create");
        let mut monster = runtime.require(&mut game, &id).expect("require");
        assert!(!gremlin_find_target(&mut monster) || monster.state.search_until > 0.0);
        gremlin_stand(&mut monster);
        gremlin_walk(&mut monster, 8.0);
        gremlin_run(&mut monster, 8.0);
    }
}
