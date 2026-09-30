//! Rerelease gladiator (`src/content/q2/rerelease/monsters/gladiator.ts`).
//!
//! Rerelease m_gladiator.cpp, including the Xatrix plasma variant.
//! ZeniMax Media, GPL-2.0.

use qa_core::math::{Bounds, length3, normalize3, sub3, vec3};

use super::common::{blocked_check_platform, check_gib, monster_flash, reacts_to_pain};
use super::tables::gladiator::{gladiator_frame, gladiator_moves};
use crate::q2::base::monsters::gladiator::gladiator_definition;
use crate::q2::missionpacks::monsters::types::mission_weapons;
use crate::q2::foundation::fields::number_field;
use crate::q2::foundation::host::{Q2Edition, Q2SoundEvent, Q2SoundLoop};
use crate::q2::foundation::monsters::ai::{
    clear_shot, corpse, enemy_body, enemy_eye, health, project_flash,
};
use crate::q2::foundation::monsters::gibs::{Q2GibOptions, throw_gib};
use crate::q2::foundation::monsters::muzzle::muzzle_offset;
use crate::q2::foundation::monsters::types::{
    MonsterContext, MonsterHandler, Q2MonsterDefinition, bind_shared_power_cells,
};
use crate::contract::{InventoryEntry, PoweredProtectionState};
use crate::q2::support::contracts::{DeathReaction, PainReaction};

/// Bind armor (`bindArmor`).
fn bind_armor(context: &mut MonsterContext) {
    bind_shared_power_cells(context);
}

/// Loop start (`initialize` shared sound).
fn loop_start(context: &mut MonsterContext, path: &str) {
    let actor = context.actor().clone();
    let origin = context.game.body_of(actor.clone()).origin;
    context.game.host_emit(
        crate::q2::foundation::host::Q2PresentationEvent::Sound(Q2SoundEvent {
            actor: Some(actor),
            origin,
            path: path.to_string(),
            channel: 1,
            volume: 1.0,
            attenuation: 1.0,
            reliable: false,
            loop_: Q2SoundLoop::Start,
            loop_owner: None,
        }),
    );
}

/// Plasma (`plasma`).
fn gladb_plasma(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let Some(eye) = enemy_eye(context) else {
        return;
    };
    let start = project_flash(context, muzzle_offset(Q2Edition::Rerelease, 61), None);
    let pos1 = context.entity().pos1;
    let direction = normalize3(sub3(pos1, start));
    let later = context.entity().frame > gladiator_frame::ATTACK3;
    let weapons = mission_weapons(&*context.game);
    weapons.fire_plasma(
        actor,
        &mut *context.game,
        start,
        direction,
        if later { 17.0 } else { 35.0 },
        725.0,
        if later { 22.0 } else { 45.0 },
        if later { 22.0 } else { 45.0 },
    );
    context.entity_mut().pos1 = eye;
}

/// Plasma check (`gladbGun_check`).
fn gladb_plasma_check(context: &mut MonsterContext) {
    if context.game.options.skill == 3 {
        gladb_plasma(context);
    }
}

/// Attack (`attack`).
fn rerelease_gladiator_attack(context: &mut MonsterContext) {
    let enemy = enemy_body(context);
    let eye = enemy_eye(context);
    let (Some(enemy), Some(eye)) = (enemy, eye) else {
        return;
    };
    let actor = context.actor().clone();
    let origin = context.game.body_of(actor).origin;
    if length3(sub3(origin, enemy.origin)) <= 112.0
        && context.state().melee_time <= context.game.host.now()
        || !clear_shot(context, muzzle_offset(Q2Edition::Rerelease, 61))
    {
        return;
    }
    context.entity_mut().pos1 = eye;
    let actor = context.actor().clone();
    let plasma = context.entity().style == 1;
    context.game.sound(
        &actor,
        if plasma {
            "weapons/plasshot.wav"
        } else {
            "gladiator/railgun.wav"
        },
        1,
        1.0,
        1.0,
    );
    context.set_move(
        if plasma {
            "gladb_move_attack_gun"
        } else {
            "gladiator_move_attack_gun"
        },
        false,
    );
}

/// Pain (`pain`).
fn rerelease_gladiator_pain(context: &mut MonsterContext, _reaction: &PainReaction) {
    let actor = context.actor().clone();
    let airborne = context.game.body_of(actor.clone()).velocity.z > 100.0;
    let max_health = context.entity().max_health;
    if health(&mut *context.game, Some(&actor)) < max_health / 2.0 {
        context.entity_mut().skin |= 1;
    } else {
        context.entity_mut().skin &= !1;
    }
    if context.game.host.now() < context.state().pain_time {
        if airborne && context.state().current_move.name == "gladiator_move_pain" {
            context.set_move("gladiator_move_pain_air", true);
        }
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    let actor = context.actor().clone();
    let path = if context.game.random() < 0.5 {
        "gladiator/pain.wav"
    } else {
        "gladiator/gldpain2.wav"
    };
    context.game.sound(&actor, path, 2, 1.0, 1.0);
    if !reacts_to_pain(context) {
        return;
    }
    context.set_move(
        if airborne {
            "gladiator_move_pain_air"
        } else {
            "gladiator_move_pain"
        },
        false,
    );
}

/// Die (`die`).
fn rerelease_gladiator_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    let actor = context.actor().clone();
    if check_gib(context) {
        context.game.sound(&actor, "misc/udeath.wav", 2, 1.0, 1.0);
        context.entity_mut().skin /= 2;
        let damage = reaction.pain.damage;
        for _ in 0..2 {
            throw_gib(
                actor.clone(),
                &mut *context.game,
                "models/objects/gibs/bone/tris.md2",
                damage,
                Q2GibOptions::default(),
            );
            throw_gib(
                actor.clone(),
                &mut *context.game,
                "models/objects/gibs/sm_meat/tris.md2",
                damage,
                Q2GibOptions::default(),
            );
            throw_gib(
                actor.clone(),
                &mut *context.game,
                "models/monsters/gladiatr/gibs/thigh.md2",
                damage,
                Q2GibOptions {
                    skinned: true,
                    ..Q2GibOptions::default()
                },
            );
        }
        for part in ["larm", "rarm", "chest", "head"] {
            let model = format!("models/monsters/gladiatr/gibs/{part}.md2");
            throw_gib(
                actor.clone(),
                &mut *context.game,
                &model,
                damage,
                Q2GibOptions {
                    skinned: true,
                    upright: part == "larm" || part == "rarm",
                    head: part == "head",
                    ..Q2GibOptions::default()
                },
            );
        }
        context.state_mut().dead = true;
        context.state_mut().gibbed = true;
        return;
    }
    if context.state().dead {
        return;
    }
    context.game.sound(&actor, "gladiator/glddeth2.wav", 4, 1.0, 1.0);
    if context.game.random() < 0.5 {
        let actor = context.actor().clone();
        context.game.sound(&actor, "gladiator/death.wav", 2, 1.0, 1.0);
    }
    context.state_mut().dead = true;
    context.state_mut().can_take_damage = true;
    let actor = context.actor().clone();
    let owned = context.game.owned_of(actor);
    context.game.set_combat_traits(
        &owned,
        &crate::q2::support::contracts::CombatTraitChanges {
            can_take_damage: Some(true),
            ..crate::q2::support::contracts::CombatTraitChanges::default()
        },
    );
    context.set_move("gladiator_move_death", true);
}

/// Melee (`GladiatorMelee`).
fn gladiator_melee(context: &mut MonsterContext) {
    let fire_hit = context.weapons.fire_hit;
    let actor = context.actor().clone();
    let min_x = context.game.body_of(actor.clone()).bounds.min.x;
    let damage = 20.0 + (context.game.random() * 5.0).floor();
    let hit = fire_hit(actor.clone(), &mut *context.game, vec3(80.0, min_x, -4.0), damage, 300.0);
    if !hit {
        let melee = context.game.host.now() + 1.5;
        context.state_mut().melee_time = melee;
    }
    context.game.sound(
        &actor,
        if hit {
            "gladiator/melee2.wav"
        } else {
            "gladiator/melee3.wav"
        },
        0,
        1.0,
        1.0,
    );
}

/// Gun (`GladiatorGun`).
fn gladiator_gun(context: &mut MonsterContext) {
    let start = project_flash(context, muzzle_offset(Q2Edition::Rerelease, 61), None);
    let pos1 = context.entity().pos1;
    let direction = normalize3(sub3(pos1, start));
    let fire_rail = context.weapons.fire_rail;
    let actor = context.actor().clone();
    fire_rail(actor, &mut *context.game, start, direction, 50.0, 100.0);
    monster_flash(context, 61, start, direction);
}

/// Shrink (`gladiator_shrink`).
fn gladiator_shrink(context: &mut MonsterContext) {
    context.entity_mut().server_flags |= 2;
    let actor = context.actor().clone();
    let mut body = context.game.body_of(actor.clone());
    body.bounds.max.z = 0.0;
    context.game.write_body(actor, &body, true);
}

/// Initialize (`initialize`).
fn rerelease_gladiator_initialize(context: &mut MonsterContext) {
    loop_start(context, "weapons/rg_hum.wav");
}

/// Gladb initialize (`initialize`).
fn gladb_initialize(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.entity_mut().style = 1;
    context.entity_mut().skin = 2;
    if !context.game.host.inventory().has(&actor) {
        let owned = context.game.owned_of(actor.clone());
        context.game.host.inventory().create(&owned, &[]);
    }
    let cells = number_field(&context.entity().spawn, "power_armor_power", 250.0);
    let armor_type = number_field(&context.entity().spawn, "power_armor_type", 2.0);
    let owned = context.game.owned_of(actor.clone());
    context.game.host.inventory().configure(
        &owned,
        &InventoryEntry {
            item: "q2:monster-power".to_string(),
            count: cells,
            capacity: 250.0f64.max(cells),
            count_policy: None,
        },
    );
    bind_armor(context);
    let owned = context.game.owned_of(actor);
    let protection = if armor_type == 0.0 {
        crate::contract::PoweredProtectionState::None
    } else if armor_type == 1.0 {
        PoweredProtectionState::Screen { cells }
    } else {
        PoweredProtectionState::Shield { cells }
    };
    context.game.host.combat().set_powered_protection(&owned, &protection);
    loop_start(context, "weapons/phaloop.wav");
}

/// Create rerelease gladiator definitions (`createRereleaseGladiatorDefinitions`).
pub fn create_rerelease_gladiator_definitions() -> Vec<Q2MonsterDefinition> {
    let mut definition = gladiator_definition();
    definition.bounds = Bounds {
        min: vec3(-32.0, -32.0, -24.0),
        max: vec3(32.0, 32.0, 42.0),
    };
    definition.moves = gladiator_moves();
    definition.blocked = Some(blocked_check_platform);
    definition.initialize = Some(MonsterHandler::Callback(rerelease_gladiator_initialize));
    definition.attack = MonsterHandler::Callback(rerelease_gladiator_attack);
    definition.pain = Some(rerelease_gladiator_pain);
    definition.die = rerelease_gladiator_die;
    definition.callbacks.insert(
        "gladiator_dead".to_string(),
        MonsterHandler::Callback(corpse),
    );
    definition.callbacks.insert(
        "GladiatorMelee".to_string(),
        MonsterHandler::Callback(gladiator_melee),
    );
    definition.callbacks.insert(
        "GladiatorGun".to_string(),
        MonsterHandler::Callback(gladiator_gun),
    );
    definition.callbacks.insert(
        "gladbGun".to_string(),
        MonsterHandler::Callback(gladb_plasma),
    );
    definition.callbacks.insert(
        "gladbGun_check".to_string(),
        MonsterHandler::Callback(gladb_plasma_check),
    );
    definition.callbacks.insert(
        "gladiator_shrink".to_string(),
        MonsterHandler::Callback(gladiator_shrink),
    );
    let mut gladb = definition.clone();
    gladb.classname = "monster_gladb".to_string();
    gladb.health = 250.0;
    gladb.mass = 350.0;
    gladb.initialize = Some(MonsterHandler::Callback(gladb_initialize));
    gladb.restore = Some(MonsterHandler::Callback(bind_armor));
    vec![definition, gladb]
}
