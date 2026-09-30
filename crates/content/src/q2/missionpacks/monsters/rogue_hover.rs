//! Rogue hover (`src/content/q2/missionpacks/monsters/hover.ts`).
//!
//! Original Rogue m_hover.c behavior. ZeniMax Media, GPL-2.0-or-later.

use super::power_armor::{
    PowerArmorKind, monster_power_armor, restore_monster_power_armor,
};
use super::rogue_common::{monster_mass, rogue_blocked_check_shot};
use super::tables::rogue_hover::{hover_frame, hover_moves};
use super::types::mission_weapons;
use crate::q2::base::monsters::common::{
    alive_enemy, begin_death, monster_loop_sound, monster_shot, standard_gib,
};
use crate::q2::base::monsters::hover::{hover_dead_think, hover_definition};
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::monsters::ai::{health, visible};
use crate::q2::foundation::monsters::types::{
    MonsterAttackState, MonsterContext, MonsterHandler, Q2MonsterDefinition,
};
use crate::q2::foundation::weapons::types::Mod;
use crate::q2::rerelease::monsters::common::monster_flash;
use crate::q2::support::contracts::{DeathReaction, PainReaction};

/// Sound path (`path`).
fn hover_sound_path(context: &mut MonsterContext, suffix: &str) -> String {
    if monster_mass(context) < 225.0 {
        format!("hover/hov{suffix}.wav")
    } else {
        format!("daedalus/daed{suffix}.wav")
    }
}

/// Sight (`sight`).
fn rogue_hover_sight(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let path = hover_sound_path(context, "sght1");
    context.game.sound(&actor, &path, 2, 1.0, 1.0);
}

/// Search (`search`).
fn rogue_hover_search(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let suffix = if context.game.random() < 0.5 { "srch1" } else { "srch2" };
    let path = hover_sound_path(context, suffix);
    context.game.sound(&actor, &path, 2, 1.0, 1.0);
}

/// Pain (`pain`).
fn rogue_hover_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    let actor = context.actor().clone();
    let max_health = context.entity().max_health;
    if health(&mut *context.game, Some(&actor)) < max_health / 2.0 {
        context.entity_mut().skin |= 1;
    }
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    if context.game.options.skill == 3 {
        return;
    }
    let skill = f64::from(context.game.options.skill);
    let first = context.game.random()
        < if reaction.damage <= 25.0 {
            0.5
        } else {
            0.45 - 0.1 * skill
        };
    let actor = context.actor().clone();
    let path = hover_sound_path(context, if first { "pain1" } else { "pain2" });
    context.game.sound(&actor, &path, 2, 1.0, 1.0);
    context.set_move(
        if first {
            if reaction.damage <= 25.0 {
                "hover_move_pain3"
            } else {
                "hover_move_pain1"
            }
        } else {
            "hover_move_pain2"
        },
        false,
    );
}

/// Die (`die`).
fn rogue_hover_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    context.entity_mut().effects = 0;
    let actor = context.actor().clone();
    let owned = context.game.owned_of(actor);
    context.game.host.combat().set_armor(
        &owned,
        &crate::contract::ArmorState {
            regular: crate::contract::RegularArmorState::None,
            powered: crate::contract::PoweredProtectionState::None,
        },
    );
    if standard_gib(
        context,
        reaction,
        2,
        2,
        "models/objects/gibs/sm_meat/tris.md2",
        1.0,
    ) || context.state().dead
    {
        return;
    }
    let suffix = if context.game.random() < 0.5 { "deth1" } else { "deth2" };
    let path = hover_sound_path(context, suffix);
    begin_death(context, reaction, &path, "hover_move_death1", 2, 4);
}

/// Blocked (`blocked`).
fn rogue_hover_blocked(context: &mut MonsterContext, _distance: f64) -> bool {
    let chance = 0.25 + 0.05 * f64::from(context.game.options.skill);
    rogue_blocked_check_shot(context, chance)
}

/// Attack (`hover_attack`).
fn hover_attack(context: &mut MonsterContext) {
    let skill = context.game.options.skill;
    let mut chance = if skill == 0 {
        0.0
    } else {
        1.0 - 0.5 / f64::from(skill)
    };
    if monster_mass(context) > 150.0 {
        chance += 0.1;
    }
    if context.game.random() > chance {
        context.state_mut().attack_state = MonsterAttackState::Straight;
        context.set_move("hover_move_attack1", true);
        return;
    }
    if context.game.random() <= 0.5 {
        let lefty = !context.state().lefty;
        context.state_mut().lefty = lefty;
    }
    context.state_mut().attack_state = MonsterAttackState::Sliding;
    context.set_move("hover_move_attack2", true);
}

/// Reattack (`hover_reattack`).
fn hover_reattack(context: &mut MonsterContext) {
    if alive_enemy(context) && visible(context, None) && context.game.random() <= 0.6 {
        match context.state().attack_state {
            MonsterAttackState::Straight => {
                context.set_move("hover_move_attack1", true);
                return;
            }
            MonsterAttackState::Sliding => {
                context.set_move("hover_move_attack2", true);
                return;
            }
            other => {
                context.game.host.diagnostic(&format!(
                    "hover_reattack: unexpected state {other:?}"
                ));
            }
        }
    }
    context.set_move("hover_move_end_attack", true);
}

/// Fire blaster (`hover_fire_blaster`).
fn hover_fire_blaster(context: &mut MonsterContext) {
    let Some((start, direction)) = monster_shot(context, 62, 0.0) else {
        return;
    };
    let daedalus = monster_mass(context) >= 200.0;
    let actor = context.actor().clone();
    if daedalus {
        let weapons = mission_weapons(&mut *context.game);
        weapons.fire_blaster2(actor, &mut *context.game, start, direction, 1.0, 1000.0, 8);
    } else {
        let effects = if context.entity().frame == hover_frame::ATTAK104 {
            64
        } else {
            0
        };
        let fire_blaster = context.weapons.fire_blaster;
        fire_blaster(
            actor,
            &mut *context.game,
            start,
            direction,
            1.0,
            1000.0,
            effects,
            false,
            Mod::BLASTER,
        );
    }
    monster_flash(context, if daedalus { 145 } else { 62 }, start, direction);
}

/// Daedalus initialize (`initialize`).
fn daedalus_initialize(context: &mut MonsterContext) {
    monster_power_armor(context, PowerArmorKind::Screen, 100.0);
    monster_loop_sound(context, "daedalus/daedidle1.wav");
}

/// Daedalus after-spawn (`afterSpawn`).
fn daedalus_after_spawn(context: &mut MonsterContext) {
    context.entity_mut().skin = 2;
    let actor = context.actor().clone();
    context.game.show(actor);
}

/// Create rogue hover definitions (`createRogueHoverDefinitions`).
pub fn create_rogue_hover_definitions() -> Vec<Q2MonsterDefinition> {
    let mut definition = hover_definition();
    definition.moves = hover_moves();
    let mut source_callbacks = Q2CallbackDefinitions::default();
    source_callbacks
        .think
        .insert("q2:rogue/hover_deadthink", hover_dead_think);
    definition.source_callbacks = Some(source_callbacks);
    definition.sight = Some(MonsterHandler::Callback(rogue_hover_sight));
    definition.search = Some(MonsterHandler::Callback(rogue_hover_search));
    definition.pain = Some(rogue_hover_pain);
    definition.die = rogue_hover_die;
    definition.blocked = Some(rogue_hover_blocked);
    definition.callbacks.insert(
        "hover_attack".to_string(),
        MonsterHandler::Callback(hover_attack),
    );
    definition.callbacks.insert(
        "hover_reattack".to_string(),
        MonsterHandler::Callback(hover_reattack),
    );
    definition.callbacks.insert(
        "hover_fire_blaster".to_string(),
        MonsterHandler::Callback(hover_fire_blaster),
    );
    let mut daedalus = definition.clone();
    daedalus.classname = "monster_daedalus".to_string();
    daedalus.kind = "daedalus".to_string();
    daedalus.health = 450.0;
    daedalus.mass = 225.0;
    daedalus.yaw_speed = Some(25.0);
    daedalus.initialize = Some(MonsterHandler::Callback(daedalus_initialize));
    daedalus.after_spawn = Some(MonsterHandler::Callback(daedalus_after_spawn));
    daedalus.restore = Some(MonsterHandler::Callback(restore_monster_power_armor));
    vec![definition, daedalus]
}
