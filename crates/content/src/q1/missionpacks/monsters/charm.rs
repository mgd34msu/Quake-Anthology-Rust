//! Hipnotic charm following and charmed target selection
//! (`src/content/q1/missionpacks/monsters/charm.ts`).

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::q1::foundation::types::{
    dot, length, normalize, vadd, vscale, vsub, yaw_for, Q1Powerup, Q1TraceRequest, POINT,
};

use super::helpers::{number, radius_actors};
use super::runtime::MissionMonster;

/// Charm owner (`charmer`).
pub fn charmer(monster: &mut MissionMonster) -> Option<ActorId> {
    if monster.entity.number("charmed") == 0.0 {
        return None;
    }
    monster.entity.references.get("charmer").cloned().flatten()
}

fn update_goal(monster: &mut MissionMonster) -> Option<ActorId> {
    monster.sync();
    let id = monster.entity.actor.id().clone();
    let owner = charmer(monster)?;
    let owner_body = monster.game.host.bodies.read(&owner)?;
    let mut goal = monster
        .entity
        .references
        .get("trigger_field")
        .cloned()
        .flatten()
        .and_then(|goal| monster.game.entity(&goal).map(|entity| entity.actor.id().clone()));
    if monster.entity.number("huntingcharmer") == 1.0 {
        let created = monster.game.create("charmed_goal", None, None).ok()?;
        monster
            .entity
            .references
            .insert("trigger_field".to_string(), Some(created.clone()));
        let _ = monster.game.set_origin(&created, owner_body.origin);
        number(monster, "huntingcharmer", 2.0);
        monster
            .entity
            .references
            .insert("goalentity".to_string(), Some(created.clone()));
        monster.flush_entity();
        goal = Some(created);
    }
    let goal = goal?;
    if monster.entity.number("huntingcharmer") == 2.0 {
        let trace = monster.game.host.trace(&Q1TraceRequest {
            start: monster.origin,
            end: owner_body.origin,
            bounds: POINT,
            ignore: Some(id),
            monsters: false,
            missile: false,
        });
        if trace.fraction == 1.0 {
            let _ = monster.game.set_origin(&goal, owner_body.origin);
        }
    } else {
        let away = vadd(
            owner_body.origin,
            vscale(normalize(vsub(monster.origin, owner_body.origin)), 300.0),
        );
        let _ = monster.game.set_origin(&goal, away);
    }
    monster.refresh();
    Some(goal)
}

/// Follow or flee the charmer (`huntCharmer`).
pub fn hunt_charmer(monster: &mut MissionMonster, flee: bool) {
    monster.sync();
    number(monster, "huntingcharmer", 1.0);
    let goal = update_goal(monster);
    if flee {
        number(monster, "huntingcharmer", 3.0);
    } else if let Some(goal) = goal {
        let origin = monster.game.body(&goal).map(|body| body.origin);
        if let Ok(origin) = origin {
            monster.entity.ideal_yaw = yaw_for(vsub(origin, monster.origin));
        }
    }
    monster.next_frame = monster.spec.walk.to_string();
    monster.flush_entity();
    monster.delay(0.1);
    monster.refresh();
}

fn stop_hunting(monster: &mut MissionMonster) {
    monster.sync();
    let goal = monster.entity.references.get("trigger_field").cloned().flatten();
    if monster.entity.number("huntingcharmer") > 1.0 {
        if let Some(goal) = goal {
            let _ = monster.game.remove(&goal);
        }
    }
    monster.entity.references.insert("goalentity".to_string(), None);
    number(monster, "huntingcharmer", 0.0);
    monster.next_frame = monster.spec.stand.to_string();
    monster.flush_entity();
    monster.delay(0.1);
    monster.refresh();
}

/// Charmed target scan (`findCharmedTarget`). Returns `None` when the
/// monster is not charmed.
pub fn find_charmed_target(monster: &mut MissionMonster) -> Option<bool> {
    monster.sync();
    let owner = charmer(monster)?;
    let id = monster.entity.actor.id().clone();
    let owner_body = monster.game.host.bodies.read(&owner)?;
    monster.entity.effects |= 8;
    monster.flush_entity();
    if monster.entity.number("huntingcharmer") > 0.0 {
        let goal = update_goal(monster);
        let distance = goal
            .as_ref()
            .and_then(|goal| monster.game.body(goal).ok())
            .map(|body| f64::from(length(vsub(monster.origin, body.origin))))
            .unwrap_or(f64::INFINITY);
        if distance < 150.0 {
            if monster.entity.number("huntingcharmer") == 3.0 && distance > 120.0 {
                monster.refresh();
                return Some(false);
            }
            stop_hunting(monster);
            monster.refresh();
            return Some(true);
        }
    } else if f64::from(length(vsub(monster.origin, owner_body.origin))) > 200.0 {
        hunt_charmer(monster, false);
        monster.refresh();
        return Some(false);
    } else if f64::from(length(vsub(monster.origin, owner_body.origin))) < 120.0 {
        hunt_charmer(monster, true);
        monster.refresh();
        return Some(false);
    }
    let mut selected: Option<ActorId> = None;
    let mut distance = 1500.0;
    for actor in radius_actors(monster.game, monster.origin, 1500.0) {
        let candidate = monster.game.entity(&actor).cloned();
        let body = monster.game.host.bodies.read(&actor);
        let (Some(candidate), Some(_body)) = (candidate, body) else {
            continue;
        };
        if candidate.movement_flags & 128 != 0
            || candidate.movement_flags & 32 == 0
            || actor == id
            || actor == owner
            || candidate.references.get("charmer").cloned().flatten() == Some(owner.clone())
            || monster.game.health(&actor) <= 0.0
            || !monster.visible(Some(&actor))
        {
            continue;
        }
        let range = monster.range_distance(Some(&actor));
        if range < distance {
            selected = Some(actor);
            distance = range;
        }
    }
    let result = match selected {
        None => false,
        Some(selected) => {
            if Some(&selected) == monster.enemy.as_ref() || distance >= 1000.0 {
                false
            } else {
                let invisible = monster
                    .game
                    .player_ref(&selected)
                    .and_then(|player| player.powerups.get(&Q1Powerup::Invisibility).copied())
                    .unwrap_or(0.0);
                if invisible > monster.game.time {
                    false
                } else {
                    monster.found(&selected);
                    true
                }
            }
        }
    };
    monster.refresh();
    Some(result)
}

/// Hipnotic target scan (`findHipnoticTarget`).
pub fn find_hipnotic_target(monster: &mut MissionMonster) -> bool {
    monster.sync();
    let id = monster.entity.actor.id().clone();
    let candidate = if monster.game.sight_entity.is_some()
        && monster.game.sight_time >= monster.game.time - 0.1
        && monster.entity.spawnflags & 3 == 0
    {
        let sight = monster.game.sight_entity.clone().expect("sight entity");
        let sight_enemy = monster
            .game
            .entity(&sight)
            .and_then(|entity| entity.monster.clone())
            .and_then(|monster| monster.enemy);
        if sight_enemy == monster.enemy {
            monster.refresh();
            return false;
        }
        Some(sight)
    } else {
        let owned = monster.entity.actor.clone();
        monster.game.host.check_client(&owned)
    };
    let Some(candidate) = candidate else {
        monster.refresh();
        return false;
    };
    if Some(&candidate) == monster.enemy.as_ref() {
        monster.refresh();
        return false;
    }
    let target = monster.game.entity(&candidate).cloned();
    let body = monster.game.host.bodies.read(&candidate);
    let Some(body) = body else {
        monster.refresh();
        return false;
    };
    if target.as_ref().map(|entity| entity.movement_flags).unwrap_or(0) & 128 != 0 {
        monster.refresh();
        return false;
    }
    let invisible = monster
        .game
        .player_ref(&candidate)
        .and_then(|player| player.powerups.get(&Q1Powerup::Invisibility).copied())
        .unwrap_or(0.0);
    if invisible > monster.game.time {
        monster.refresh();
        return false;
    }
    let distance = monster.range_distance(Some(&candidate));
    if distance >= 1000.0 || !monster.visible(Some(&candidate)) {
        monster.refresh();
        return false;
    }
    if distance >= 120.0 {
        let hostile = monster
            .game
            .player_ref(&candidate)
            .map(|player| player.hostile_until)
            .or_else(|| target.as_ref().map(|entity| entity.number("show_hostile")))
            .unwrap_or(0.0);
        let angles = monster
            .game
            .body(&id)
            .map(|body| body.angles)
            .unwrap_or(Vec3 { x: 0.0, y: 0.0, z: 0.0 });
        let basis = monster.game.make_vectors(angles);
        if (distance >= 500.0 || hostile < monster.game.time)
            && dot(normalize(vsub(body.origin, monster.origin)), basis.forward) <= 0.3
        {
            monster.refresh();
            return false;
        }
    }
    let mut candidate = candidate;
    if target.as_ref().map(|entity| entity.number("charmed")).unwrap_or(0.0) == 0.0
        && !monster.game.is_player(&candidate)
    {
        let redirected = target
            .as_ref()
            .and_then(|entity| entity.monster.clone())
            .and_then(|monster| monster.enemy);
        let Some(redirected) = redirected else {
            monster.refresh();
            return false;
        };
        if !monster.game.is_player(&redirected) {
            monster.refresh();
            return false;
        }
        candidate = redirected;
    }
    monster.found(&candidate);
    monster.refresh();
    true
}

/// Charmed walk steering (`walkWithCharmer`).
pub fn walk_with_charmer(monster: &mut MissionMonster, distance: f64) -> bool {
    monster.sync();
    if charmer(monster).is_none() {
        monster.refresh();
        return false;
    }
    if monster.find_target() {
        monster.refresh();
        return true;
    }
    let goal = monster
        .entity
        .references
        .get("goalentity")
        .cloned()
        .flatten()
        .and_then(|goal| monster.game.entity(&goal).map(|entity| entity.actor.id().clone()))
        .or_else(|| monster.game.find(&monster.state.path).first().cloned());
    if let Some(goal) = goal {
        let owned = monster.entity.actor.clone();
        monster.game.host.move_to_goal(&owned, &goal, distance, None);
    }
    if monster.entity.number("huntingcharmer") != 0.0 {
        let remaining = (monster.entity.next_think - monster.game.time) / 2.0;
        monster.delay(remaining);
    }
    monster.refresh();
    true
}

#[cfg(test)]
mod tests {
    use super::super::helpers::HULL_BOUNDS;
    use super::super::runtime::Q1MissionPackMonsters;
    use super::super::types::{MissionMonsterHooks, PackMonsterDefinition};
    use super::*;
    use crate::q1::base::species::{MonsterMovement, MonsterSpecies};
    use crate::q1::foundation::entity::Q1MonsterSpecies;
    use crate::q1::missionpacks::types::{test_game, Q1MissionPack};
    use std::sync::Arc;

    fn test_definition() -> PackMonsterDefinition {
        PackMonsterDefinition {
            spec: Box::leak(Box::new(MonsterSpecies {
                species: Q1MonsterSpecies::Dog,
                kill_string: None,
                classnames: &["monster_dog"],
                model: "dog",
                head: None,
                health: 25.0,
                gib_health: -35.0,
                gibs: &[],
                bounds: HULL_BOUNDS,
                stand: "dog_stand1",
                walk: "dog_walk1",
                run: "dog_run1",
                sight: "",
                missile: None,
                melee: true,
                movement: MonsterMovement::Walk,
            })),
            base_behavior: false,
            frames: &[],
            actions: Vec::new(),
            callbacks: Vec::new(),
            spawn: None,
            start: None,
            pain: Arc::new(|_, _, _| {}),
            die: Arc::new(|_, _| {}),
            melee: None,
            check_attack: None,
            found: None,
            ai: None,
            use_: None,
        }
    }

    #[test]
    fn uncharmed_monster_has_no_charmer() {
        let mut game = test_game();
        let mut runtime =
            Q1MissionPackMonsters::new(&mut game, Q1MissionPack::Hipnotic, MissionMonsterHooks::default())
                .expect("new");
        runtime.register(&mut game, test_definition()).expect("register");
        let id = game.create("monster_dog", None, None).expect("create");
        let mut monster = runtime.require(&mut game, &id).expect("require");
        assert!(charmer(&mut monster).is_none());
        assert!(find_charmed_target(&mut monster).is_none());
        assert!(!walk_with_charmer(&mut monster, 8.0));
        assert!(!find_hipnotic_target(&mut monster));
    }
}
