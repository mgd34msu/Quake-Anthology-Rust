//! Q1 addon grunt (`src/content/q1/addons/monsters/ordinary/army.ts`).
//!
//! `quakec_{mg1,mg3}/monsters/soldier.qc` and `fight.qc`
//! SoldierCheckAttack. GPL-2.0-or-later.

use std::collections::HashMap;
use std::sync::OnceLock;

use qa_core::identity::{same_actor, ActorId};
use qa_core::math::{Bounds, Vec3};

use crate::q1::base::monsters::{BaseMonster, MonsterActionHandler};
use crate::q1::base::species::MonsterMovement;
use crate::q1::base::species::MonsterSpecies;
use crate::q1::foundation::entity::Q1MonsterSpecies;
use crate::q1::foundation::types::{normalize, vscale, vsub, Q1SoundChannel, POINT};
use crate::q1::foundation::weapons::fire_bullets;
use crate::q1::{q1_error, Q1Error};

use crate::q1::addons::monsters::ai::Mg3Monster;

/// Grunt spawn defaults (`armySpecies`).
pub const ARMY_SPEC: MonsterSpecies = MonsterSpecies {
    species: Q1MonsterSpecies::Army,
    kill_string: None,
    classnames: &["monster_army"],
    model: "soldier",
    head: Some("h_guard"),
    health: 30.0,
    gib_health: -35.0,
    gibs: &["gib1", "gib2", "gib3"],
    bounds: Bounds {
        min: Vec3 {
            x: -16.0,
            y: -16.0,
            z: -24.0,
        },
        max: Vec3 {
            x: 16.0,
            y: 16.0,
            z: 40.0,
        },
    },
    stand: "army_stand1",
    walk: "army_walk1",
    run: "army_run1",
    sight: "soldier/sight1.wav",
    missile: Some("army_atk1"),
    melee: false,
    movement: MonsterMovement::Walk,
};

fn army_fire(monster: &mut BaseMonster) -> Result<(), Q1Error> {
    let enemy = monster.monster.enemy.clone();
    let Some(enemy) = enemy else {
        return Ok(());
    };
    let Some(target) = monster.game.host.bodies.read(&enemy) else {
        return Ok(());
    };
    monster.face()?;
    let id = monster.id.clone();
    monster
        .game
        .sound(&id, "soldier/sattck1.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
    let origin = monster.origin()?;
    let angles = monster.game.body(&id).map(|body| body.angles)?;
    let owned = monster
        .game
        .entity_ref(&id)
        .map(|entity| entity.actor.clone())
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let lead = vsub(target.origin, vscale(target.velocity, 0.2));
    fire_bullets(
        monster.game,
        &owned,
        normalize(vsub(lead, origin)),
        angles,
        4,
        0.1,
        0.1,
        None,
    );
    monster.game.update_entity(&id, |entity| {
        entity.effects |= 2;
    })
}

fn army_refire(monster: &mut BaseMonster) -> Result<(), Q1Error> {
    if monster.game.options().skill == 3 && !monster.monster.refired && monster.visible(None)? {
        monster.monster.refired = true;
        monster.controller.next_frame = String::from("army_atk1");
    }
    Ok(())
}

/// Grunt frame actions (`armyActions`).
pub fn army_actions() -> &'static HashMap<String, MonsterActionHandler> {
    static ACTIONS: OnceLock<HashMap<String, MonsterActionHandler>> = OnceLock::new();
    ACTIONS.get_or_init(|| {
        HashMap::from([
            (String::from("army_fire"), army_fire as MonsterActionHandler),
            (String::from("army_refire"), army_refire as MonsterActionHandler),
        ])
    })
}

/// Attempt a grunt attack (`armyAttack`).
pub fn army_attack(monster: &mut Mg3Monster) -> Result<bool, Q1Error> {
    let enemy = monster.monster.monster.enemy.clone();
    let range = monster
        .monster
        .game
        .world
        .clone()
        .and_then(|world| {
            monster
                .monster
                .game
                .entity_ref(&world)
                .map(|entity| entity.number("enemy_range"))
        })
        .unwrap_or(0.0);
    let Some(enemy) = enemy else {
        return Ok(false);
    };
    let start = monster.monster.eye(None)?;
    let end = monster.monster.eye(Some(&enemy))?;
    let (Some(start), Some(end)) = (start, end) else {
        return Ok(false);
    };
    let trace = monster
        .monster
        .game
        .host
        .trace(&crate::q1::foundation::types::Q1TraceRequest {
            start,
            end,
            bounds: POINT,
            ignore: Some(monster.monster.id.clone()),
            monsters: true,
            missile: false,
        });
    let chance = if range == 0.0 {
        0.9
    } else if range == 1.0 {
        0.4
    } else if range == 2.0 {
        0.05
    } else {
        0.0
    };
    if trace.actor.as_ref().is_none_or(|actor| !same_actor(actor, &enemy))
        || trace.in_open && trace.in_water
        || range == 3.0
        || monster.monster.game.time < monster.monster.monster.attack_finished
        || monster.monster.game.host.random() >= chance
    {
        return Ok(false);
    }
    monster.play("army_atk1")?;
    let delay = 1.0 + monster.monster.game.host.random();
    monster.attack_finished(delay);
    if monster.monster.game.host.random() < 0.3 {
        monster.monster.controller.lefty = !monster.monster.controller.lefty;
        let lefty = monster.monster.controller.lefty;
        monster
            .monster
            .game
            .update_entity(&monster.monster.id.clone(), |entity| {
                entity.fields.insert(
                    String::from("lefty"),
                    if lefty { String::from("1") } else { String::from("0") },
                );
            })?;
    }
    Ok(true)
}

/// React to grunt pain (`armyPain`).
pub fn army_pain(
    monster: &mut Mg3Monster,
    attacker: Option<&ActorId>,
    damage: f64,
    nightmare_resistance: bool,
) -> Result<(), Q1Error> {
    let attacker = attacker.cloned();
    monster.retaliate(attacker.as_ref())?;
    if monster.monster.monster.pain_finished > monster.monster.game.time
        || nightmare_resistance
            && monster.monster.game.options().skill > 2
            && monster.monster.game.host.random() * 100.0 > damage
    {
        return Ok(());
    }
    let rolled = monster.monster.game.host.random();
    monster.monster.monster.pain_finished = monster.monster.game.time + if rolled < 0.2 { 0.6 } else { 1.1 };
    monster.play(if rolled < 0.2 {
        "army_pain1"
    } else if rolled < 0.6 {
        "army_painb1"
    } else {
        "army_painc1"
    })?;
    monster.monster.game.sound(
        &monster.monster.id.clone(),
        if rolled < 0.2 {
            "soldier/pain1.wav"
        } else {
            "soldier/pain2.wav"
        },
        Q1SoundChannel::Voice,
        1.0,
        1.0,
    )
}
