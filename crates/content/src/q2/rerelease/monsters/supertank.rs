//! Rerelease supertank (`src/content/q2/rerelease/monsters/supertank.ts`).
//!
//! ZeniMax Media, GPL-2.0-or-later.

use qa_core::math::{normalize3, sub3, vec3};

use super::boss::{boss_explode, boss_explode_think};
use super::common::{
    blocked_check_platform, calculate_pitch_to_fire, chainfist, check_gib, monster_flash, predict_aim,
    predicted_direction, reacts_to_pain,
};
use super::tables::flashes::rerelease_flash;
use super::tables::supertank::{supertank_frame, supertank_moves};
use crate::contract::{InventoryEntry, PoweredProtectionState};
use crate::q2::base::monsters::supertank::supertank_definition;
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::fields::number_field;
use crate::q2::foundation::host::{Q2Edition, Q2EffectEvent, Q2PresentationEvent, Q2SoundEvent, Q2SoundLoop};
use crate::q2::foundation::monsters::ai::{
    clear_shot, enemy_body, enemy_eye, health, project_flash, target_distance, visible,
};
use crate::q2::foundation::monsters::gibs::{throw_gib, Q2GibOptions};
use crate::q2::foundation::monsters::muzzle::muzzle_offset;
use crate::q2::foundation::monsters::types::{
    bind_shared_power_cells, MonsterContext, MonsterHandler, Q2MonsterDefinition,
};
use crate::q2::foundation::weapons::types::Q2GrenadeAdjustment;
use crate::q2::missionpacks::monsters::types::mission_weapons;
use crate::q2::support::contracts::{CombatTraitChanges, DeathReaction, PainReaction};

/// Bind armor (`bindArmor`).
fn supertank_bind_armor(context: &mut MonsterContext) {
    bind_shared_power_cells(context);
}

/// Gib (`gib`).
fn supertank_gib(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let origin = context.game.body_of(actor.clone()).origin;
    context.game.host_emit(Q2PresentationEvent::Effect(Q2EffectEvent {
        effect: "q2:explosion1-big".to_string(),
        origin,
        direction: vec3(0.0, 0.0, 0.0),
        count: 1,
        color: 0,
    }));
    let sound = context.game.require_entity(&actor).sound.clone();
    if !sound.is_empty() {
        context.game.host_emit(Q2PresentationEvent::Sound(Q2SoundEvent {
            actor: Some(actor.clone()),
            origin,
            path: sound,
            channel: 0,
            volume: 1.0,
            attenuation: 1.0,
            reliable: false,
            loop_: Q2SoundLoop::Stop,
            loop_owner: None,
        }));
    }
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
    for part in ["cgun", "chest", "core", "ltread", "rgun", "rtread", "tube", "head"] {
        throw_gib(
            actor.clone(),
            &mut *context.game,
            &format!("models/monsters/boss1/gibs/{part}.md2"),
            500.0,
            Q2GibOptions {
                skinned: true,
                metallic: part == "cgun" || part == "head",
                upright: ["ltread", "rgun", "rtread", "tube"].contains(&part),
                head: part == "head",
            },
        );
    }
    context.state_mut().gibbed = true;
}

/// Shared initialize (`initialize`).
fn supertank_initialize_inner(context: &mut MonsterContext, is_n64: bool) {
    let actor = context.actor().clone();
    if is_n64 {
        let entity = context.game.require_entity_mut(&actor);
        entity.spawnflags |= 16;
        entity.count = 10;
    }
    if context.game.require_entity(&actor).spawnflags & 8 == 0 {
        return;
    }
    if !context.game.host.inventory().has(&actor) {
        let owned = context.game.owned_of(actor.clone());
        context.game.host.inventory().create(&owned, &[]);
    }
    let spawn = context.game.require_entity(&actor).spawn.clone();
    let cells = number_field(&spawn, "power_armor_power", 400.0);
    let armor_type = number_field(&spawn, "power_armor_type", 2.0);
    let owned = context.game.owned_of(actor.clone());
    context.game.host.inventory().configure(
        &owned,
        &InventoryEntry {
            item: "q2:monster-power".to_string(),
            count: cells,
            capacity: 400.0_f64.max(cells),
            count_policy: None,
        },
    );
    supertank_bind_armor(context);
    let owned = context.game.owned_of(actor);
    let protection = if armor_type == 0.0 {
        PoweredProtectionState::None
    } else if armor_type == 1.0 {
        PoweredProtectionState::Screen { cells }
    } else {
        PoweredProtectionState::Shield { cells }
    };
    context.game.host.combat().set_powered_protection(&owned, &protection);
}

/// Initialize (`initialize`).
fn supertank_initialize(context: &mut MonsterContext) {
    supertank_initialize_inner(context, false);
}

/// N64 initialize (`initialize`).
fn supertank_initialize_n64(context: &mut MonsterContext) {
    supertank_initialize_inner(context, true);
}

/// Boss5 initialize (`monster_boss5 initialize`).
fn supertank_boss5_initialize(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.require_entity_mut(&actor).spawnflags |= 8;
    supertank_initialize_inner(context, false);
    context.game.require_entity_mut(&actor).skin = 2;
}

/// N64 boss5 initialize (`monster_boss5 initialize`).
fn supertank_boss5_initialize_n64(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.require_entity_mut(&actor).spawnflags |= 8;
    supertank_initialize_inner(context, true);
    context.game.require_entity_mut(&actor).skin = 2;
}

/// Restore (`restore`).
fn supertank_restore(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    if context.game.require_entity(&actor).spawnflags & 8 != 0 {
        supertank_bind_armor(context);
    }
}

/// Attack (`attack`).
fn supertank_attack(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let Some(enemy) = enemy_body(context) else {
        return;
    };
    let range = target_distance(context);
    let delta = sub3(enemy.origin, context.game.body_of(actor.clone()).origin);
    let chain = clear_shot(context, muzzle_offset(Q2Edition::Rerelease, 64));
    let rocket = clear_shot(context, muzzle_offset(Q2Edition::Rerelease, 70));
    let grenade = clear_shot(
        context,
        muzzle_offset(Q2Edition::Rerelease, rerelease_flash::SUPERTANK_GRENADE_1 as usize),
    );
    if chain && (!rocket || range <= 540.0 || context.game.random() < 0.3) {
        if grenade && (range >= 350.0 || delta.z > 120.0 || context.game.random() < 0.2) {
            context.set_move("supertank_move_attack4", true);
            return;
        }
        let now = context.game.host.now();
        let delay = 1.5 + context.game.random() * 1.2;
        context.game.require_entity_mut(&actor).timestamp = now + delay;
        context.set_move("supertank_move_attack1", true);
        return;
    }
    if rocket {
        let grenade_attack = grenade && (delta.z > 120.0 || context.game.random() < 0.2);
        context.set_move(
            if grenade_attack {
                "supertank_move_attack4"
            } else {
                "supertank_move_attack2"
            },
            true,
        );
        return;
    }
    if grenade {
        context.set_move("supertank_move_attack4", true);
    }
}

/// Pain (`pain`).
fn supertank_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    let actor = context.actor().clone();
    let max_health = context.game.require_entity(&actor).max_health;
    let skin = context.game.require_entity(&actor).skin;
    let bloodied = health(&mut *context.game, Some(&actor)) < max_health / 2.0;
    context.game.require_entity_mut(&actor).skin = if bloodied { skin | 1 } else { skin & !1 };
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let damage = reaction.damage;
    if !chainfist(context) {
        if damage <= 25.0 && context.game.random() < 0.2 {
            return;
        }
        let frame = context.game.require_entity(&actor).frame;
        if (supertank_frame::ATTAK2_1..=supertank_frame::ATTAK2_14).contains(&frame) {
            return;
        }
    }
    context.game.sound(
        &actor,
        if damage <= 10.0 {
            "bosstank/btkpain1.wav"
        } else if damage <= 25.0 {
            "bosstank/btkpain3.wav"
        } else {
            "bosstank/btkpain2.wav"
        },
        2,
        1.0,
        1.0,
    );
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    if !reacts_to_pain(context) {
        return;
    }
    context.set_move(
        if damage <= 10.0 {
            "supertank_move_pain1"
        } else if damage <= 25.0 {
            "supertank_move_pain2"
        } else {
            "supertank_move_pain3"
        },
        true,
    );
}

/// Die (`die`).
fn supertank_die(context: &mut MonsterContext, _reaction: &DeathReaction) {
    let actor = context.actor().clone();
    if context.game.require_entity(&actor).spawnflags & (1 << 16) != 0 {
        if check_gib(context) {
            supertank_gib(context);
            context.state_mut().dead = true;
            return;
        }
        if context.state().dead {
            return;
        }
    } else {
        context.game.sound(&actor, "bosstank/btkdeth1.wav", 2, 1.0, 1.0);
        context.state_mut().dead = true;
        context.state_mut().can_take_damage = false;
        let owned = context.game.owned_of(actor.clone());
        context.game.set_combat_traits(
            &owned,
            &CombatTraitChanges {
                can_take_damage: Some(false),
                ..CombatTraitChanges::default()
            },
        );
    }
    context.set_move("supertank_move_death", true);
}

/// Boss loop (`BossLoop`).
fn supertank_boss_loop(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    if context.game.require_entity(&actor).spawnflags & 16 == 0 {
        return;
    }
    let entity = context.game.require_entity_mut(&actor);
    if entity.count != 0 {
        entity.count -= 1;
    } else {
        entity.spawnflags &= !16;
    }
    context.state_mut().next_frame = supertank_frame::DEATH_19;
}

/// Dead (`supertank_dead`).
fn supertank_dead(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    if context.game.require_entity(&actor).spawnflags & (1 << 16) == 0 {
        supertank_gib(context);
        return;
    }
    context.state_mut().dead = false;
    context.state_mut().can_take_damage = true;
    let owned = context.game.owned_of(actor);
    context.game.set_combat_traits(
        &owned,
        &CombatTraitChanges {
            can_take_damage: Some(true),
            ..CombatTraitChanges::default()
        },
    );
}

/// Reattack (`supertank_reattack1`).
fn supertank_reattack1(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let timestamp = context.game.require_entity(&actor).timestamp;
    let again = visible(context, None) && (timestamp >= context.game.host.now() || context.game.random() < 0.3);
    context.set_move(
        if again {
            "supertank_move_attack1"
        } else {
            "supertank_move_end_attack1"
        },
        true,
    );
}

/// Machine gun (`supertankMachineGun`).
fn supertank_machine_gun(context: &mut MonsterContext) {
    if enemy_body(context).is_none() {
        return;
    }
    let actor = context.actor().clone();
    let frame = context.game.require_entity(&actor).frame;
    let id = 64 + frame - supertank_frame::ATTAK1_1;
    let yaw = context.game.body_of(actor.clone()).angles.y;
    let start = project_flash(
        context,
        muzzle_offset(Q2Edition::Rerelease, id as usize),
        Some(vec3(0.0, yaw, 0.0)),
    );
    let Some(direction) = predicted_direction(context, start, 0.0, true, -0.1) else {
        return;
    };
    let fire_bullet = context.weapons.fire_bullet;
    fire_bullet(actor, &mut *context.game, start, direction, 6.0, 4.0, 900.0, 1500.0, 0);
    monster_flash(context, id, start, direction);
}

/// Rocket (`supertankRocket`).
fn supertank_rocket(context: &mut MonsterContext) {
    let Some(enemy) = enemy_eye(context) else {
        return;
    };
    let actor = context.actor().clone();
    let frame = context.game.require_entity(&actor).frame;
    let id = if frame == supertank_frame::ATTAK2_8 {
        70
    } else if frame == supertank_frame::ATTAK2_11 {
        71
    } else {
        72
    };
    let start = project_flash(context, muzzle_offset(Q2Edition::Rerelease, id as usize), None);
    let heat = context.game.require_entity(&actor).spawnflags & 8 != 0;
    let direction = if heat {
        Some(normalize3(sub3(enemy, start)))
    } else {
        predicted_direction(context, start, 750.0, false, 0.0)
    };
    let Some(direction) = direction else {
        return;
    };
    if heat {
        let weapons = mission_weapons(&*context.game);
        weapons.fire_heat_rocket(
            actor,
            &mut *context.game,
            start,
            direction,
            40.0,
            500.0,
            60.0,
            40.0,
            None,
        );
    } else {
        let fire_rocket = context.weapons.fire_rocket;
        fire_rocket(actor, &mut *context.game, start, direction, 50.0, 750.0, 70.0, 50.0);
    }
    monster_flash(context, id, start, direction);
}

/// Grenade (`supertankGrenade`).
fn supertank_grenade(context: &mut MonsterContext) {
    if enemy_body(context).is_none() {
        return;
    }
    let actor = context.actor().clone();
    let frame = context.game.require_entity(&actor).frame;
    let id = if frame == supertank_frame::ATTAK4_1 {
        rerelease_flash::SUPERTANK_GRENADE_1
    } else {
        rerelease_flash::SUPERTANK_GRENADE_2
    };
    let start = project_flash(context, muzzle_offset(Q2Edition::Rerelease, id as usize), None);
    let spread = (context.game.random() * 2.0 - 1.0) * 0.1;
    let Some(target) = predict_aim(context, start, 0.0, false, spread) else {
        return;
    };
    let mut speed = 500.0;
    while speed < 1000.0 {
        let Some(direction) =
            calculate_pitch_to_fire(context, target.point, start, target.direction, speed, 2.5, true, false)
        else {
            speed += 100.0;
            continue;
        };
        let gravity = context.game.host.gravity();
        let fire_grenade = context.weapons.fire_grenade;
        fire_grenade(
            actor.clone(),
            &mut *context.game,
            start,
            direction,
            50.0,
            speed,
            2.5,
            90.0,
            false,
            false,
            true,
            Some(Q2GrenadeAdjustment {
                right: 0.0,
                up: 0.0,
                gravity,
            }),
        );
        monster_flash(context, id, start, direction);
        return;
    }
}

/// Create the rerelease supertank definitions (`createRereleaseSupertankDefinitions`).
pub fn create_rerelease_supertank_definitions(is_n64: bool) -> Vec<Q2MonsterDefinition> {
    let mut definition = supertank_definition();
    definition.moves = supertank_moves();
    definition.blocked = Some(blocked_check_platform);
    let mut source_callbacks = Q2CallbackDefinitions::default();
    source_callbacks.think.insert("BossExplode_think", boss_explode_think);
    definition.source_callbacks = Some(source_callbacks);
    definition.initialize = Some(MonsterHandler::Callback(if is_n64 {
        supertank_initialize_n64
    } else {
        supertank_initialize
    }));
    definition.restore = Some(MonsterHandler::Callback(supertank_restore));
    definition.attack = MonsterHandler::Callback(supertank_attack);
    definition.pain = Some(supertank_pain);
    definition.die = supertank_die;
    for (name, handler) in [
        ("BossExplode", MonsterHandler::Callback(boss_explode)),
        ("BossLoop", MonsterHandler::Callback(supertank_boss_loop)),
        ("supertank_dead", MonsterHandler::Callback(supertank_dead)),
        ("supertank_reattack1", MonsterHandler::Callback(supertank_reattack1)),
        ("supertankMachineGun", MonsterHandler::Callback(supertank_machine_gun)),
        ("supertankRocket", MonsterHandler::Callback(supertank_rocket)),
        ("supertankGrenade", MonsterHandler::Callback(supertank_grenade)),
    ] {
        definition.callbacks.insert(name.to_string(), handler);
    }
    let mut boss5 = definition.clone();
    boss5.classname = "monster_boss5".to_string();
    boss5.initialize = Some(MonsterHandler::Callback(if is_n64 {
        supertank_boss5_initialize_n64
    } else {
        supertank_boss5_initialize
    }));
    vec![definition, boss5]
}
