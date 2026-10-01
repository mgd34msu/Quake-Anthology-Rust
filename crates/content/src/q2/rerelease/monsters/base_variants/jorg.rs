//! Rerelease jorg (`src/content/q2/rerelease/monsters/base-variants/jorg.ts`).
//!
//! ZeniMax Media, GPL-2.0-or-later.

use super::super::boss::{boss_explode, boss_explode_think};
use super::super::common::{chainfist, monster_flash, predicted_direction, reacts_to_pain};
use super::super::tables::boss31::boss31_frame;
use super::super::tables::boss31::boss31_moves;
use super::makron::toss_rerelease_makron;
use crate::q2::base::monsters::boss_common::stop_loop;
use crate::q2::base::monsters::common::{monster_loop_sound, monster_shot};
use crate::q2::base::monsters::jorg::create_jorg_definition;
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::host::{Q2Edition, Q2EffectEvent, Q2PresentationEvent};
use crate::q2::foundation::monsters::ai::{health, project_flash, visible};
use crate::q2::foundation::monsters::gibs::{throw_gib, Q2GibOptions};
use crate::q2::foundation::monsters::muzzle::muzzle_offset;
use crate::q2::foundation::monsters::perception::{check_attack_with_profile, Q2AttackChanceProfile};
use crate::q2::foundation::monsters::types::{MonsterContext, MonsterHandler, Q2MonsterDefinition};
use crate::q2::support::contracts::{CombatTraitChanges, DeathReaction, PainReaction};
use qa_core::math::vec3;

/// End sound (`endSound`).
fn jorg_end_sound(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    if !context.game.require_entity(&actor).sound.is_empty() {
        context.game.sound(&actor, "boss3/bs3atck1_end.wav", 1, 1.0, 1.0);
        context.game.require_entity_mut(&actor).sound = String::new();
        stop_loop(context);
    }
}

/// Run (`run`).
fn rerelease_jorg_run(context: &mut MonsterContext) {
    let stand_ground = context.state().stand_ground;
    context.set_move(
        if stand_ground {
            "jorg_move_stand"
        } else {
            "jorg_move_run"
        },
        true,
    );
    jorg_end_sound(context);
}

/// Stand (`stand`).
fn rerelease_jorg_stand(context: &mut MonsterContext) {
    context.set_move("jorg_move_stand", true);
    jorg_end_sound(context);
}

/// Bullet (`bullet`).
fn jorg_bullet(context: &mut MonsterContext, flash: usize, offset: f64) {
    let start = project_flash(context, muzzle_offset(Q2Edition::Rerelease, flash), None);
    let Some(direction) = predicted_direction(context, start, 0.0, false, offset) else {
        return;
    };
    let actor = context.actor().clone();
    let fire_bullet = context.weapons.fire_bullet;
    fire_bullet(actor, &mut *context.game, start, direction, 6.0, 4.0, 300.0, 500.0, 0);
    monster_flash(context, flash as i32, start, direction);
}

/// Fire bullet left (`jorg_firebullet_left`).
fn jorg_firebullet_left(context: &mut MonsterContext) {
    jorg_bullet(context, 120, 0.2);
}

/// Fire bullet right (`jorg_firebullet_right`).
fn jorg_firebullet_right(context: &mut MonsterContext) {
    jorg_bullet(context, 126, -0.2);
}

/// Fire bullet (`jorg_firebullet`).
fn jorg_firebullet(context: &mut MonsterContext) {
    jorg_bullet(context, 120, 0.2);
    jorg_bullet(context, 126, -0.2);
}

/// Toss (`toss`).
fn jorg_toss(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let spawned = toss_rerelease_makron(context);
    if let Some(transfer) = context.game.monsters.hooks.healthbar_transfer {
        transfer(actor, spawned, &mut *context.game);
    }
}

/// Initialize (`initialize`).
fn rerelease_jorg_initialize(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.require_entity_mut(&actor).model2 = "models/monsters/boss3/rider/tris.md2".to_string();
    context.state_mut().ignore_shots = true;
}

/// Check attack (`checkAttack`).
fn rerelease_jorg_check_attack(context: &mut MonsterContext) -> bool {
    check_attack_with_profile(
        context,
        &Q2AttackChanceProfile {
            stand_ground: 0.4,
            melee: 0.8,
            near: 0.4,
            mid: 0.2,
            far: 0.0,
            strafe_scalar: 0.0,
        },
    )
}

/// Attack (`attack`).
fn rerelease_jorg_attack(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    if context.game.random() <= 0.75 {
        context.game.sound(&actor, "boss3/bs3atck1.wav", 1, 1.0, 1.0);
        context.game.require_entity_mut(&actor).sound = "boss3/w_loop.wav".to_string();
        monster_loop_sound(context, "boss3/w_loop.wav");
        context.set_move("jorg_move_start_attack1", true);
        return;
    }
    context.game.sound(&actor, "boss3/bs3atck2.wav", 2, 1.0, 1.0);
    context.set_move("jorg_move_attack2", true);
}

/// Pain (`pain`).
fn rerelease_jorg_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    let actor = context.actor().clone();
    let max_health = context.game.require_entity(&actor).max_health;
    let frame = context.game.require_entity(&actor).frame;
    let bloodied = health(&mut *context.game, Some(&actor)) < max_health / 2.0;
    context.game.require_entity_mut(&actor).skin = if bloodied { 1 } else { 0 };
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let damage = reaction.damage;
    if !chainfist(context) {
        if damage <= 40.0 && context.game.random() <= 0.6 {
            return;
        }
        if (boss31_frame::ATTAK101..=boss31_frame::ATTAK108).contains(&frame) && context.game.random() <= 0.005 {
            return;
        }
        if (boss31_frame::ATTAK109..=boss31_frame::ATTAK114).contains(&frame) && context.game.random() <= 0.00005 {
            return;
        }
        if (boss31_frame::ATTAK201..=boss31_frame::ATTAK208).contains(&frame) && context.game.random() <= 0.005 {
            return;
        }
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    let mut heavy = false;
    if damage > 50.0 && damage <= 100.0 {
        context.game.sound(&actor, "boss3/bs3pain2.wav", 2, 1.0, 1.0);
    } else if damage > 100.0 && context.game.random() <= 0.3 {
        heavy = true;
        context.game.sound(&actor, "boss3/bs3pain3.wav", 2, 1.0, 1.0);
    }
    if !reacts_to_pain(context) {
        return;
    }
    jorg_end_sound(context);
    if damage <= 50.0 {
        context.set_move("jorg_move_pain1", true);
    } else if damage <= 100.0 {
        context.set_move("jorg_move_pain2", true);
    } else if heavy {
        context.set_move("jorg_move_pain3", true);
    }
}

/// Die (`die`).
fn rerelease_jorg_die(context: &mut MonsterContext, _reaction: &DeathReaction) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "boss3/bs3deth1.wav", 2, 1.0, 1.0);
    jorg_end_sound(context);
    context.state_mut().dead = true;
    context.state_mut().can_take_damage = false;
    context.game.require_entity_mut(&actor).count = 0;
    let owned = context.game.owned_of(actor);
    context.game.set_combat_traits(
        &owned,
        &CombatTraitChanges {
            can_take_damage: Some(false),
            ..CombatTraitChanges::default()
        },
    );
    context.set_move("jorg_move_death", true);
}

/// Reattack (`jorg_reattack1`).
fn jorg_reattack1(context: &mut MonsterContext) {
    if visible(context, None) && context.game.random() < 0.9 {
        context.set_move("jorg_move_attack1", true);
        return;
    }
    context.set_move("jorg_move_end_attack1", true);
    jorg_end_sound(context);
}

/// BFG (`jorgBFG`).
fn jorg_bfg(context: &mut MonsterContext) {
    let Some((start, direction)) = monster_shot(context, 132, 0.0) else {
        return;
    };
    let actor = context.actor().clone();
    context.game.sound(&actor, "makron/bfg_fire.wav", 1, 1.0, 1.0);
    let fire_bfg = context.weapons.fire_bfg;
    fire_bfg(actor, &mut *context.game, start, direction, 50.0, 300.0, 200.0);
    monster_flash(context, 132, start, direction);
}

/// Dead (`jorg_dead`).
fn jorg_dead(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let origin = context.game.body_of(actor.clone()).origin;
    context.game.host_emit(Q2PresentationEvent::Effect(Q2EffectEvent {
        effect: "q2:explosion1-big".to_string(),
        origin,
        direction: vec3(0.0, 0.0, 0.0),
        count: 1,
        color: 0,
    }));
    stop_loop(context);
    let entity = context.game.require_entity_mut(&actor);
    entity.sound = String::new();
    entity.skin /= 2;
    for _ in 0..2 {
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/objects/gibs/sm_meat/tris.md2",
            500.0,
            Q2GibOptions::default(),
        );
    }
    for _ in 0..2 {
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/objects/gibs/sm_metal/tris.md2",
            500.0,
            Q2GibOptions {
                metallic: true,
                ..Q2GibOptions::default()
            },
        );
    }
    for part in [
        "chest", "foot", "foot", "tube", "tube", "tube", "tube", "spike", "spike", "spike", "spike", "spike", "spike",
    ] {
        throw_gib(
            actor.clone(),
            &mut *context.game,
            &format!("models/monsters/boss3/jorg/gibs/{part}.md2"),
            500.0,
            Q2GibOptions {
                skinned: true,
                ..Q2GibOptions::default()
            },
        );
    }
    for part in ["gun", "gun", "thigh", "thigh", "spine"] {
        throw_gib(
            actor.clone(),
            &mut *context.game,
            &format!("models/monsters/boss3/jorg/gibs/{part}.md2"),
            500.0,
            Q2GibOptions {
                skinned: true,
                upright: true,
                ..Q2GibOptions::default()
            },
        );
    }
    throw_gib(
        actor.clone(),
        &mut *context.game,
        "models/monsters/boss3/jorg/gibs/head.md2",
        500.0,
        Q2GibOptions {
            skinned: true,
            metallic: true,
            head: true,
            ..Q2GibOptions::default()
        },
    );
    context.state_mut().gibbed = true;
    jorg_toss(context);
}

/// Create the rerelease jorg definition (`createRereleaseJorgDefinition`).
pub fn create_rerelease_jorg_definition() -> Q2MonsterDefinition {
    let mut definition = create_jorg_definition();
    definition.moves = boss31_moves();
    definition.model = "models/monsters/boss3/jorg/tris.md2".to_string();
    definition.health = 8000.0;
    definition.run = MonsterHandler::Callback(rerelease_jorg_run);
    let mut source_callbacks = Q2CallbackDefinitions::default();
    source_callbacks.think.insert("BossExplode_think", boss_explode_think);
    definition.source_callbacks = Some(source_callbacks);
    definition.initialize = Some(MonsterHandler::Callback(rerelease_jorg_initialize));
    definition.stand = MonsterHandler::Callback(rerelease_jorg_stand);
    definition.check_attack = Some(rerelease_jorg_check_attack);
    definition.attack = MonsterHandler::Callback(rerelease_jorg_attack);
    definition.pain = Some(rerelease_jorg_pain);
    definition.die = rerelease_jorg_die;
    for (name, handler) in [
        ("jorg_run", MonsterHandler::Callback(rerelease_jorg_run)),
        ("BossExplode", MonsterHandler::Callback(boss_explode)),
        ("jorg_reattack1", MonsterHandler::Callback(jorg_reattack1)),
        ("jorg_attack1_end_sound", MonsterHandler::Callback(jorg_end_sound)),
        ("jorgBFG", MonsterHandler::Callback(jorg_bfg)),
        ("jorg_firebullet_left", MonsterHandler::Callback(jorg_firebullet_left)),
        ("jorg_firebullet_right", MonsterHandler::Callback(jorg_firebullet_right)),
        ("jorg_firebullet", MonsterHandler::Callback(jorg_firebullet)),
        ("MakronToss", MonsterHandler::Callback(jorg_toss)),
        ("jorg_dead", MonsterHandler::Callback(jorg_dead)),
    ] {
        definition.callbacks.insert(name.to_string(), handler);
    }
    definition
}
