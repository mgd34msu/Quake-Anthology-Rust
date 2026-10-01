//! Xatrix heavy soldier (`src/content/q2/missionpacks/monsters/soldierh.ts`).
//!
//! Quake II xatrix/m_soldier.c heavy soldier variants.
//! ZeniMax Media, GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::{add3, normalize3, scale3, sub3};

use super::dabeam::monster_dabeam;
use super::tables::xatrix_soldierh::{soldierh_frame, soldierh_moves};
use super::types::mission_weapons;
use crate::q2::base::monsters::common::{
    HUMANOID_BOUNDS, alive_enemy, finish_corpse_default, monster_muzzle,
};
use crate::q2::foundation::fields::integer_field;
use crate::q2::foundation::monsters::ai::{
    angles_vectors, enemy_body, enemy_eye, health, project_flash, set_duck,
    target_distance, vector_angles,
};
use crate::q2::foundation::monsters::gibs::{Q2GibOptions, throw_gib, throw_head};
use crate::q2::foundation::monsters::muzzle::muzzle_offset;
use crate::q2::foundation::monsters::types::{
    MonsterContext, MonsterHandler, Q2MonsterDefinition, record_at,
};
use crate::q2::support::contracts::{DeathReaction, PainReaction, TraceResult};

/// Blaster flashes.
const BLASTER_FLASHES: [usize; 8] = [39, 40, 83, 86, 89, 92, 95, 98];
/// Machinegun flashes.
const MACHINEGUN_FLASHES: [usize; 8] = [43, 44, 85, 88, 91, 94, 97, 100];

/// Stand (`stand`).
fn soldierh_stand(context: &mut MonsterContext) {
    if context.state().current_move.name == "soldierh_move_stand3"
        || context.game.random() < 0.8
    {
        context.set_move("soldierh_move_stand1", true);
    } else {
        context.set_move("soldierh_move_stand3", true);
    }
}

/// Run (`run`).
fn soldierh_run(context: &mut MonsterContext) {
    if context.state().stand_ground {
        context.set_move("soldierh_move_stand1", true);
        return;
    }
    let current = context.state().current_move.name.clone();
    context.set_move(
        if current == "soldierh_move_walk1"
            || current == "soldierh_move_walk2"
            || current == "soldierh_move_start_run"
        {
            "soldierh_move_run"
        } else {
            "soldierh_move_start_run"
        },
        false,
    );
}

/// Refire (`refire`).
fn soldierh_refire(context: &mut MonsterContext) -> bool {
    context.game.options.skill == 3 && context.game.random() < 0.5
        || target_distance(context) < 80.0
}

/// Duck (`duck`).
fn soldierh_duck_down(context: &mut MonsterContext) {
    if context.state().ducked {
        return;
    }
    set_duck(context, true);
    let pause = context.game.host.now() + 1.0;
    context.state_mut().pause_time = pause;
}

/// Pain or death sound (`painSound`).
fn soldierh_pain_sound(context: &mut MonsterContext, death: bool) {
    let n = context.entity().skin | 1;
    let variant = if n == 1 {
        2
    } else if n == 3 {
        1
    } else {
        3
    };
    let actor = context.actor().clone();
    let path = format!(
        "soldier/{}{variant}.wav",
        if death { "soldeth" } else { "solpain" }
    );
    context.game.sound(&actor, &path, 2, 1.0, 1.0);
}

/// Fire (`fire`).
fn soldierh_fire(context: &mut MonsterContext, index: usize) {
    let actor = context.actor().clone();
    let skin = context.entity().skin;
    let flashes = if skin < 4 {
        &BLASTER_FLASHES
    } else {
        &MACHINEGUN_FLASHES
    };
    let flash = *record_at(flashes, index);
    let edition = context.game.options.edition;
    let start = project_flash(context, muzzle_offset(edition, flash), None);
    let body = context.game.body_of(actor.clone());
    let axes = angles_vectors(body.angles);
    let mut direction = axes.forward;
    if index != 5 && index != 6 {
        let Some(eye) = enemy_eye(context) else {
            return;
        };
        let aim = angles_vectors(vector_angles(sub3(eye, start)));
        let rx = context.game.random() * 2.0 - 1.0;
        let rz = context.game.random() * 2.0 - 1.0;
        direction = normalize3(add3(
            scale3(aim.forward, 8192.0),
            add3(
                scale3(aim.right, (rx * 100.0) as f32),
                scale3(aim.up, (rz * 50.0) as f32),
            ),
        ));
    }
    if skin < 2 {
        let weapons = mission_weapons(&mut *context.game);
        weapons.fire_ion_ripper(actor, &mut *context.game, start, direction, 5.0, 600.0, 0x100000);
        monster_muzzle(context, flash as i32, direction, start);
        return;
    }
    if skin < 4 {
        let weapons = mission_weapons(&mut *context.game);
        weapons.fire_blue_blaster(actor, &mut *context.game, start, direction, 1.0, 600.0, 0x400000);
        monster_muzzle(context, 17, direction, start);
        return;
    }
    if !context.state().hold_frame {
        let pause = context.game.host.now()
            + (3.0 + (context.game.random() * 8.0).floor()) * 0.1;
        context.state_mut().pause_time = pause;
    }
    if context.game.random() > 0.8 {
        let actor = context.actor().clone();
        context.game.sound(&actor, "misc/lasfly.wav", 0, 1.0, 3.0);
    }
    let Some(enemy) = enemy_body(context) else {
        return;
    };
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    let aim_angles = vector_angles(sub3(enemy.origin, body.origin));
    let aim = angles_vectors(aim_angles);
    let offset = muzzle_offset(context.game.options.edition, flash);
    let origin = add3(
        body.origin,
        add3(
            scale3(aim.right, offset.x + if flash == 85 { -14.0 } else { 2.0 }),
            add3(
                scale3(aim.up, offset.z + 8.0),
                scale3(aim.forward, offset.y),
            ),
        ),
    );
    let target = context.entity().enemy.clone();
    monster_dabeam(&actor, &mut *context.game, target, origin, aim_angles, 1.0, false);
    let hold = context.game.host.now() < context.state().pause_time;
    context.state_mut().hold_frame = hold;
}

/// Hyper refire (`hyperRefire`).
fn soldierh_hyper_refire_inner(context: &mut MonsterContext, second: bool) {
    let skin = context.entity().skin;
    if skin < 2 || skin >= 4 {
        return;
    }
    if context.game.random() < 0.7 {
        context.entity_mut().frame = if second {
            soldierh_frame::ATTAK205
        } else {
            soldierh_frame::ATTAK103
        };
    } else {
        let actor = context.actor().clone();
        context.game.sound(&actor, "weapons/hyprbd1a.wav", 0, 1.0, 1.0);
    }
}

/// Walk (`walk`).
fn soldierh_walk(context: &mut MonsterContext) {
    let first = context.game.random() < 0.5;
    context.set_move(
        if first {
            "soldierh_move_walk1"
        } else {
            "soldierh_move_walk2"
        },
        false,
    );
}

/// Attack (`attack`).
fn soldierh_attack(context: &mut MonsterContext) {
    let skin = context.entity().skin;
    let first = context.game.random() < 0.5;
    context.set_move(
        if skin >= 4 {
            "soldierh_move_attack4"
        } else if first {
            "soldierh_move_attack1"
        } else {
            "soldierh_move_attack2"
        },
        false,
    );
}

/// After spawn (`afterSpawn`).
fn soldierh_after_spawn(context: &mut MonsterContext) {
    // Xatrix sets variant health after the common monster_start max_health copy.
    let health = integer_field(&context.entity().spawn, "health", 0);
    context.entity_mut().max_health = f64::from(health);
}

/// Sight (`sight`).
fn soldierh_sight(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let path = if context.game.random() < 0.5 {
        "soldier/solsght1.wav"
    } else {
        "soldier/solsrch1.wav"
    };
    context.game.sound(&actor, path, 2, 1.0, 1.0);
    if context.game.options.skill > 0
        && target_distance(context) >= 500.0
        && context.game.random() > 0.5
    {
        context.set_move(
            if context.entity().skin < 4 {
                "soldierh_move_attack6"
            } else {
                "soldierh_move_attack4"
            },
            false,
        );
    }
}

/// Pain (`pain`).
fn soldierh_pain(context: &mut MonsterContext, _reaction: &PainReaction) {
    let actor = context.actor().clone();
    let max_health = context.entity().max_health;
    if health(&mut *context.game, Some(&actor)) < max_health / 2.0 {
        context.entity_mut().skin |= 1;
    }
    let airborne = context.game.body_of(actor).velocity.z > 100.0;
    if context.game.host.now() < context.state().pain_time {
        let current = context.state().current_move.name.clone();
        if airborne
            && (current == "soldierh_move_pain1"
                || current == "soldierh_move_pain2"
                || current == "soldierh_move_pain3")
        {
            context.set_move("soldierh_move_pain4", true);
        }
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    soldierh_pain_sound(context, false);
    if airborne {
        context.set_move("soldierh_move_pain4", true);
        return;
    }
    if context.game.options.skill == 3 {
        return;
    }
    let random = context.game.random();
    context.set_move(
        if random < 0.33 {
            "soldierh_move_pain1"
        } else if random < 0.66 {
            "soldierh_move_pain2"
        } else {
            "soldierh_move_pain3"
        },
        false,
    );
}

/// Die (`die`).
fn soldierh_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    let actor = context.actor().clone();
    if health(&mut *context.game, Some(&actor)) <= context.state().gib_health {
        let actor = context.actor().clone();
        context.game.sound(&actor, "misc/udeath.wav", 2, 1.0, 1.0);
        let damage = reaction.pain.damage;
        for _ in 0..3 {
            throw_gib(
                actor.clone(),
                &mut *context.game,
                "models/objects/gibs/sm_meat/tris.md2",
                damage,
                Q2GibOptions::default(),
            );
        }
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/objects/gibs/chest/tris.md2",
            damage,
            Q2GibOptions::default(),
        );
        throw_head(
            actor,
            &mut *context.game,
            "models/objects/gibs/head2/tris.md2",
            damage,
        );
        context.state_mut().dead = true;
        context.state_mut().gibbed = true;
        return;
    }
    if context.state().dead {
        return;
    }
    context.state_mut().dead = true;
    context.state_mut().can_take_damage = true;
    context.entity_mut().skin |= 1;
    let actor = context.actor().clone();
    let owned = context.game.owned_of(actor);
    context.game.set_combat_traits(
        &owned,
        &crate::q2::support::contracts::CombatTraitChanges {
            can_take_damage: Some(true),
            ..crate::q2::support::contracts::CombatTraitChanges::default()
        },
    );
    soldierh_pain_sound(context, true);
    let actor = context.actor().clone();
    let origin = context.game.body_of(actor).origin;
    let view_height = f64::from(context.entity().view_height);
    if (origin.z as f64 + view_height - f64::from(reaction.point.z)).abs() <= 4.0 {
        context.set_move("soldierh_move_death3", true);
        return;
    }
    let deaths = [
        "soldierh_move_death1",
        "soldierh_move_death2",
        "soldierh_move_death4",
        "soldierh_move_death5",
        "soldierh_move_death6",
    ];
    let pick = *record_at(&deaths, (context.game.random() * 5.0).floor() as usize);
    context.set_move(pick, true);
}

/// Dodge (`dodge`).
fn soldierh_dodge(
    context: &mut MonsterContext,
    attacker: &ActorId,
    eta: f64,
    _trace: Option<&TraceResult>,
    _direct: bool,
) {
    if context.game.random() > 0.25 {
        return;
    }
    if context.entity().enemy.is_none() {
        context.entity_mut().enemy = Some(attacker.clone());
    }
    if context.game.options.skill == 0 {
        context.set_move("soldierh_move_duck", true);
        return;
    }
    let pause = context.game.host.now() + eta + 0.3;
    context.state_mut().pause_time = pause;
    let skill = context.game.options.skill;
    let duck = context.game.random() > if skill == 1 { 0.33 } else { 0.66 };
    context.set_move(
        if duck {
            "soldierh_move_duck"
        } else {
            "soldierh_move_attack3"
        },
        false,
    );
}

/// Idle (`soldierh_idle`).
fn soldierh_idle(context: &mut MonsterContext) {
    if context.game.random() > 0.8 {
        let actor = context.actor().clone();
        context.game.sound(&actor, "soldier/solidle1.wav", 2, 1.0, 2.0);
    }
}

/// Cock (`soldierh_cock`).
fn soldierh_cock(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let attenuation = if context.entity().frame == soldierh_frame::STAND322 {
        2.0
    } else {
        1.0
    };
    context.game.sound(&actor, "infantry/infatck3.wav", 1, 1.0, attenuation);
}

/// Walk1 random (`soldierh_walk1_random`).
fn soldierh_walk1_random(context: &mut MonsterContext) {
    if context.game.random() > 0.1 {
        context.state_mut().next_frame = soldierh_frame::WALK101;
    }
}

/// Hyper sound (`soldierh_hyper_sound`).
fn soldierh_hyper_sound(context: &mut MonsterContext) {
    let skin = context.entity().skin;
    if skin >= 2 && skin < 4 {
        let actor = context.actor().clone();
        context.game.sound(&actor, "weapons/hyprbl1a.wav", 0, 1.0, 1.0);
    }
}

/// Hyper refire callbacks.
fn soldierh_hyper_refire1(context: &mut MonsterContext) {
    soldierh_hyper_refire_inner(context, false);
}
fn soldierh_hyper_refire2(context: &mut MonsterContext) {
    soldierh_hyper_refire_inner(context, true);
}

/// Fire callbacks.
fn soldierh_fire1(context: &mut MonsterContext) {
    soldierh_fire(context, 0);
}
fn soldierh_fire2(context: &mut MonsterContext) {
    soldierh_fire(context, 1);
}
fn soldierh_fire3(context: &mut MonsterContext) {
    soldierh_duck_down(context);
    soldierh_fire(context, 2);
}
fn soldierh_fire4(context: &mut MonsterContext) {
    soldierh_fire(context, 3);
}
fn soldierh_fire8(context: &mut MonsterContext) {
    soldierh_fire(context, 7);
}
fn soldierh_fire6(context: &mut MonsterContext) {
    if context.entity().skin < 4 {
        soldierh_fire(context, 5);
    }
}
fn soldierh_fire7(context: &mut MonsterContext) {
    if context.entity().skin < 4 {
        soldierh_fire(context, 6);
    }
}
fn soldierh_ripper1(context: &mut MonsterContext) {
    if context.entity().skin < 4 {
        soldierh_fire(context, 0);
    }
}
fn soldierh_ripper2(context: &mut MonsterContext) {
    if context.entity().skin < 4 {
        soldierh_fire(context, 1);
    }
}

/// Attack1 refire 1 (`soldierh_attack1_refire1`).
fn soldierh_attack1_refire1(context: &mut MonsterContext) {
    if context.entity().skin <= 1 && alive_enemy(context) {
        context.state_mut().next_frame = if soldierh_refire(context) {
            soldierh_frame::ATTAK102
        } else {
            soldierh_frame::ATTAK110
        };
    }
}

/// Attack1 refire 2 (`soldierh_attack1_refire2`).
fn soldierh_attack1_refire2(context: &mut MonsterContext) {
    if context.entity().skin >= 2 && alive_enemy(context) && soldierh_refire(context) {
        context.state_mut().next_frame = soldierh_frame::ATTAK102;
    }
}

/// Attack2 refire 1 (`soldierh_attack2_refire1`).
fn soldierh_attack2_refire1(context: &mut MonsterContext) {
    if context.entity().skin <= 1 && alive_enemy(context) {
        context.state_mut().next_frame = if soldierh_refire(context) {
            soldierh_frame::ATTAK204
        } else {
            soldierh_frame::ATTAK216
        };
    }
}

/// Attack2 refire 2 (`soldierh_attack2_refire2`).
fn soldierh_attack2_refire2(context: &mut MonsterContext) {
    if context.entity().skin >= 2 && alive_enemy(context) {
        let skill = context.game.options.skill;
        if skill == 3 && context.game.random() < 0.5
            || target_distance(context) < 80.0 && context.entity().skin < 4
        {
            context.state_mut().next_frame = soldierh_frame::ATTAK204;
        }
    }
}

/// Attack3 refire (`soldierh_attack3_refire`).
fn soldierh_attack3_refire(context: &mut MonsterContext) {
    if context.game.host.now() + 0.4 < context.state().pause_time {
        context.state_mut().next_frame = soldierh_frame::ATTAK303;
    }
}

/// Attack6 refire (`soldierh_attack6_refire`).
fn soldierh_attack6_refire(context: &mut MonsterContext) {
    if alive_enemy(context)
        && target_distance(context) >= 500.0
        && context.game.options.skill == 3
    {
        context.state_mut().next_frame = soldierh_frame::RUNS03;
    }
}

/// Duck up (`soldierh_duck_up`).
fn soldierh_duck_up(context: &mut MonsterContext) {
    set_duck(context, false);
}

/// Duck hold (`soldierh_duck_hold`).
fn soldierh_duck_hold(context: &mut MonsterContext) {
    let hold = context.game.host.now() < context.state().pause_time;
    context.state_mut().hold_frame = hold;
}

/// Develop a heavy soldier definition (`common` + variant overrides).
fn soldierh_definition(classname: &str, health: f64, skin: i32) -> Q2MonsterDefinition {
    let mut definition = Q2MonsterDefinition::new(
        classname,
        "soldierh",
        "models/monsters/soldierh/tris.md2",
        health,
        -30.0,
        100.0,
        HUMANOID_BOUNDS,
        1.0,
        "soldierh_move_stand3",
        soldierh_moves(),
        MonsterHandler::Callback(soldierh_stand),
        MonsterHandler::Callback(soldierh_walk),
        MonsterHandler::Callback(soldierh_run),
        MonsterHandler::Callback(soldierh_attack),
        soldierh_die,
    );
    definition.after_spawn = Some(MonsterHandler::Callback(soldierh_after_spawn));
    definition.sight = Some(MonsterHandler::Callback(soldierh_sight));
    definition.pain = Some(soldierh_pain);
    definition.dodge = Some(soldierh_dodge);
    if skin >= 0 {
        definition.initialize = Some(MonsterHandler::Callback(soldierh_hyper_initialize(skin)));
    }
    let mut callbacks = definition.callbacks.clone();
    callbacks.insert(
        "soldierh_stand".to_string(),
        MonsterHandler::Callback(soldierh_stand),
    );
    callbacks.insert(
        "soldierh_run".to_string(),
        MonsterHandler::Callback(soldierh_run),
    );
    callbacks.insert(
        "soldierh_dead".to_string(),
        MonsterHandler::Callback(finish_corpse_default),
    );
    callbacks.insert(
        "soldierh_idle".to_string(),
        MonsterHandler::Callback(soldierh_idle),
    );
    callbacks.insert(
        "soldierh_cock".to_string(),
        MonsterHandler::Callback(soldierh_cock),
    );
    callbacks.insert(
        "soldierh_walk1_random".to_string(),
        MonsterHandler::Callback(soldierh_walk1_random),
    );
    callbacks.insert(
        "soldierh_hyper_sound".to_string(),
        MonsterHandler::Callback(soldierh_hyper_sound),
    );
    callbacks.insert(
        "soldierh_hyper_refire1".to_string(),
        MonsterHandler::Callback(soldierh_hyper_refire1),
    );
    callbacks.insert(
        "soldierh_hyper_refire2".to_string(),
        MonsterHandler::Callback(soldierh_hyper_refire2),
    );
    callbacks.insert(
        "soldierh_fire1".to_string(),
        MonsterHandler::Callback(soldierh_fire1),
    );
    callbacks.insert(
        "soldierh_fire2".to_string(),
        MonsterHandler::Callback(soldierh_fire2),
    );
    callbacks.insert(
        "soldierh_fire3".to_string(),
        MonsterHandler::Callback(soldierh_fire3),
    );
    callbacks.insert(
        "soldierh_fire4".to_string(),
        MonsterHandler::Callback(soldierh_fire4),
    );
    callbacks.insert(
        "soldierh_fire8".to_string(),
        MonsterHandler::Callback(soldierh_fire8),
    );
    callbacks.insert(
        "soldierh_fire6".to_string(),
        MonsterHandler::Callback(soldierh_fire6),
    );
    callbacks.insert(
        "soldierh_fire7".to_string(),
        MonsterHandler::Callback(soldierh_fire7),
    );
    callbacks.insert(
        "soldierh_ripper1".to_string(),
        MonsterHandler::Callback(soldierh_ripper1),
    );
    callbacks.insert(
        "soldierh_ripper2".to_string(),
        MonsterHandler::Callback(soldierh_ripper2),
    );
    callbacks.insert(
        "soldierh_attack1_refire1".to_string(),
        MonsterHandler::Callback(soldierh_attack1_refire1),
    );
    callbacks.insert(
        "soldierh_attack1_refire2".to_string(),
        MonsterHandler::Callback(soldierh_attack1_refire2),
    );
    callbacks.insert(
        "soldierh_attack2_refire1".to_string(),
        MonsterHandler::Callback(soldierh_attack2_refire1),
    );
    callbacks.insert(
        "soldierh_attack2_refire2".to_string(),
        MonsterHandler::Callback(soldierh_attack2_refire2),
    );
    callbacks.insert(
        "soldierh_attack3_refire".to_string(),
        MonsterHandler::Callback(soldierh_attack3_refire),
    );
    callbacks.insert(
        "soldierh_attack6_refire".to_string(),
        MonsterHandler::Callback(soldierh_attack6_refire),
    );
    callbacks.insert(
        "soldierh_duck_down".to_string(),
        MonsterHandler::Callback(soldierh_duck_down),
    );
    callbacks.insert(
        "soldierh_duck_up".to_string(),
        MonsterHandler::Callback(soldierh_duck_up),
    );
    callbacks.insert(
        "soldierh_duck_hold".to_string(),
        MonsterHandler::Callback(soldierh_duck_hold),
    );
    definition.callbacks = callbacks;
    definition
}

/// Hypergun initialize (`initialize`).
fn soldierh_hyper_initialize(skin: i32) -> fn(&mut MonsterContext) {
    if skin == 2 {
        soldierh_initialize_hyper
    } else {
        soldierh_initialize_laser
    }
}

/// Hypergun skin initialize.
fn soldierh_initialize_hyper(context: &mut MonsterContext) {
    context.entity_mut().skin = 2;
}

/// Lasergun skin initialize.
fn soldierh_initialize_laser(context: &mut MonsterContext) {
    context.entity_mut().skin = 4;
}

/// Create heavy soldier definitions (`createSoldierHeavyDefinitions`).
pub fn create_soldier_heavy_definitions() -> Vec<Q2MonsterDefinition> {
    vec![
        soldierh_definition("monster_soldier_ripper", 50.0, -1),
        soldierh_definition("monster_soldier_hypergun", 60.0, 2),
        soldierh_definition("monster_soldier_lasergun", 70.0, 4),
    ]
}
