//! Q1 mg3 ordinary attacks (`src/content/q1/addons/monsters/ordinary/attack.ts`).
//!
//! `quakec_mg3/ai.qc` CheckAnyAttack and species attack policies.
//! GPL-2.0-or-later.

use qa_core::identity::same_actor;

use crate::q1::foundation::entity::Q1AttackState;
use crate::q1::foundation::types::{length, vsub, Q1TraceRequest, POINT};
use crate::q1::Q1Error;

use super::army::army_attack;
use crate::q1::addons::monsters::ai::Mg3Monster;

/// Attempt an ordinary mg3 attack (`mg3OrdinaryAttack`).
pub fn mg3_ordinary_attack(monster: &mut Mg3Monster) -> Result<bool, Q1Error> {
    let enemy = monster.monster.monster.enemy.clone();
    let world_number = |name: &str| {
        monster
            .monster
            .game
            .world
            .clone()
            .and_then(|world| {
                monster
                    .monster
                    .game
                    .entity_ref(&world)
                    .map(|entity| entity.number(name))
            })
            .unwrap_or(0.0)
    };
    if world_number("enemy_visible") == 0.0 || enemy.is_none() {
        return Ok(false);
    }
    let enemy = enemy.ok_or_else(|| crate::q1::q1_error("Missing Q1 entity"))?;
    let range = world_number("enemy_range");
    let classname = monster
        .monster
        .game
        .entity_ref(&monster.monster.id.clone())
        .map(|entity| entity.classname.clone())
        .unwrap_or_default();
    if classname == "monster_army" {
        return army_attack(monster);
    }
    if classname == "monster_demon1" || classname == "monster_dog" {
        if range == 0.0 {
            monster
                .monster
                .game
                .update_entity(&monster.monster.id.clone(), |entity| {
                    entity.attack_state = Q1AttackState::Melee;
                })?;
            return Ok(true);
        }
        let body = monster.monster.game.body(&monster.monster.id.clone())?;
        let Some(target) = monster.monster.game.host.bodies.read(&enemy) else {
            return Ok(false);
        };
        let height = f64::from(target.bounds.max.z - target.bounds.min.z);
        if f64::from(body.origin.z + body.bounds.min.z)
            > f64::from(target.origin.z + target.bounds.min.z) + height * 0.75
            || f64::from(body.origin.z + body.bounds.max.z)
                < f64::from(target.origin.z + target.bounds.min.z) + height * 0.25
        {
            return Ok(false);
        }
        let target_origin = monster.monster.target()?.unwrap_or(target.origin);
        let delta = vsub(target_origin, body.origin);
        let distance = f64::from(delta.x).hypot(f64::from(delta.y));
        let demon = classname == "monster_demon1";
        if demon {
            if distance < 100.0 || distance > 200.0 && monster.monster.game.host.random() < 0.9 {
                return Ok(false);
            }
        } else if distance < 80.0 || distance > 150.0 {
            return Ok(false);
        }
        monster
            .monster
            .game
            .update_entity(&monster.monster.id.clone(), |entity| {
                entity.attack_state = Q1AttackState::Missile;
            })?;
        if demon {
            monster
                .monster
                .game
                .sound_simple(&monster.monster.id.clone(), "demon/djump.wav")?;
        }
        return Ok(true);
    }
    let shambler = classname == "monster_shambler";
    let ogre = classname == "monster_ogre";
    let wizard = classname == "monster_wizard";
    if !shambler && !ogre && !wizard {
        return monster.check_attack();
    }
    if !wizard && range == 0.0 && monster.monster.game.can_damage(&enemy, &monster.monster.id.clone()) {
        monster
            .monster
            .game
            .update_entity(&monster.monster.id.clone(), |entity| {
                entity.attack_state = Q1AttackState::Melee;
            })?;
        return Ok(true);
    }
    if monster.monster.game.time < monster.monster.monster.attack_finished {
        return Ok(false);
    }
    if wizard && range == 3.0 {
        mg3_wizard_straight(monster)?;
        return Ok(false);
    }
    let start = monster.monster.eye(None)?;
    let end = monster.monster.eye(Some(&enemy))?;
    let (Some(start), Some(end)) = (start, end) else {
        return Ok(false);
    };
    if shambler && f64::from(length(vsub(start, end))) > 600.0 {
        return Ok(false);
    }
    let trace = monster.monster.game.host.trace(&Q1TraceRequest {
        start,
        end,
        bounds: POINT,
        ignore: Some(monster.monster.id.clone()),
        monsters: true,
        missile: false,
    });
    if trace.actor.as_ref().is_none_or(|actor| !same_actor(actor, &enemy)) || !wizard && trace.in_open && trace.in_water
    {
        if wizard {
            mg3_wizard_straight(monster)?;
        }
        return Ok(false);
    }
    if !wizard {
        if range == 3.0 {
            return Ok(false);
        }
        monster
            .monster
            .game
            .update_entity(&monster.monster.id.clone(), |entity| {
                entity.attack_state = Q1AttackState::Missile;
            })?;
        let delay = (if shambler { 2.0 } else { 1.0 }) + 2.0 * monster.monster.game.host.random();
        monster.attack_finished(delay);
        return Ok(true);
    }
    let chance = if range == 0.0 {
        0.9
    } else if range == 1.0 {
        0.6
    } else if range == 2.0 {
        0.2
    } else {
        0.0
    };
    if monster.monster.game.host.random() < chance {
        monster.monster.controller.sliding = false;
        monster
            .monster
            .game
            .update_entity(&monster.monster.id.clone(), |entity| {
                entity.attack_state = Q1AttackState::Missile;
            })?;
        return Ok(true);
    }
    if range == 2.0 {
        mg3_wizard_straight(monster)?;
    } else if !monster.monster.controller.sliding {
        monster.monster.controller.sliding = true;
        monster
            .monster
            .game
            .update_entity(&monster.monster.id.clone(), |entity| {
                entity.attack_state = Q1AttackState::Straight;
            })?;
        monster.play("wiz_side1")?;
    }
    Ok(false)
}

/// Return a wizard to its run (`straight`).
fn mg3_wizard_straight(monster: &mut Mg3Monster) -> Result<(), Q1Error> {
    let attack_state = monster
        .monster
        .game
        .entity_ref(&monster.monster.id.clone())
        .map(|entity| entity.attack_state)
        .unwrap_or(Q1AttackState::Straight);
    if !monster.monster.controller.sliding && attack_state == Q1AttackState::Straight {
        return Ok(());
    }
    monster.monster.controller.sliding = false;
    monster
        .monster
        .game
        .update_entity(&monster.monster.id.clone(), |entity| {
            entity.attack_state = Q1AttackState::Straight;
        })?;
    monster.play("wiz_run1")
}
