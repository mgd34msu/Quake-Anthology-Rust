//! Rerelease hover (`src/content/q2/rerelease/monsters/base-variants/hover.ts`).
//!
//! ZeniMax Media, GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::vec3;

use super::super::common::{check_gib, monster_flash, reacts_to_pain, rerelease_random};
use super::super::tables::flashes::rerelease_flash;
use super::super::tables::hover::hover_moves;
use crate::contract::PoweredProtectionState;
use crate::q2::base::monsters::common::{alive_enemy, monster_loop_sound, monster_shot};
use crate::q2::base::monsters::hover::hover_definition;
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::host::{Q2EffectEvent, Q2GameServices, Q2MotionKind, Q2PresentationEvent};
use crate::q2::foundation::monsters::ai::{health, visible};
use crate::q2::foundation::monsters::gibs::{throw_gib, Q2GibOptions};
use crate::q2::foundation::monsters::types::{MonsterAttackState, MonsterContext, MonsterHandler, Q2MonsterDefinition};
use crate::q2::foundation::weapons::types::Mod;
use crate::q2::support::contracts::{CombatTraitChanges, DeathReaction, PainReaction};

/// Gib (`gib`).
fn hover_gib(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let origin = context.game.body_of(actor.clone()).origin;
    context.game.host_emit(Q2PresentationEvent::Effect(Q2EffectEvent {
        effect: "q2:explosion1".to_string(),
        origin,
        direction: vec3(0.0, 0.0, 0.0),
        count: 1,
        color: 0,
    }));
    context.game.require_entity_mut(&actor).skin /= 2;
    for _ in 0..2 {
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/objects/gibs/sm_meat/tris.md2",
            150.0,
            Q2GibOptions::default(),
        );
    }
    for _ in 0..2 {
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/objects/gibs/sm_metal/tris.md2",
            150.0,
            Q2GibOptions {
                metallic: true,
                ..Q2GibOptions::default()
            },
        );
    }
    throw_gib(
        actor.clone(),
        &mut *context.game,
        "models/monsters/hover/gibs/chest.md2",
        150.0,
        Q2GibOptions {
            skinned: true,
            ..Q2GibOptions::default()
        },
    );
    for _ in 0..2 {
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/monsters/hover/gibs/ring.md2",
            150.0,
            Q2GibOptions {
                skinned: true,
                metallic: true,
                ..Q2GibOptions::default()
            },
        );
    }
    for _ in 0..2 {
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/monsters/hover/gibs/foot.md2",
            150.0,
            Q2GibOptions {
                skinned: true,
                ..Q2GibOptions::default()
            },
        );
    }
    throw_gib(
        actor.clone(),
        &mut *context.game,
        "models/monsters/hover/gibs/head.md2",
        150.0,
        Q2GibOptions {
            skinned: true,
            head: true,
            ..Q2GibOptions::default()
        },
    );
    context.state_mut().dead = true;
    context.state_mut().gibbed = true;
}

/// Dead think (`deadThink`).
fn hover_dead_think(actor: ActorId, game: &mut Q2GameServices) {
    if game.body_of(actor.clone()).ground.is_none() && game.host.now() < game.require_entity(&actor).timestamp {
        game.schedule(actor, 0.1, hover_dead_think);
        return;
    }
    if !game.monsters.states.contains_key(&actor) {
        return;
    }
    let mut context = MonsterContext::new(actor, game);
    hover_gib(&mut context);
}

/// Initialize (`initialize`).
fn rerelease_hover_initialize(context: &mut MonsterContext) {
    let state = context.state_mut();
    state.yaw_speed = 18.0;
    state.alternate_fly = true;
    state.fly_thrusters = false;
    state.fly_acceleration = 20.0;
    state.fly_speed = 120.0;
    state.fly_min_distance = 150.0;
    state.fly_max_distance = 350.0;
    monster_loop_sound(context, "hover/hovidle1.wav");
}

/// Pain (`pain`).
fn rerelease_hover_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    let actor = context.actor().clone();
    let max_health = context.game.require_entity(&actor).max_health;
    let skin = context.game.require_entity(&actor).skin;
    let bloodied = health(&mut *context.game, Some(&actor)) < max_health / 2.0;
    context.game.require_entity_mut(&actor).skin = if bloodied { skin | 1 } else { skin & !1 };
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    let first = context.game.random() < 0.5;
    context.game.sound(
        &actor,
        if first {
            "hover/hovpain1.wav"
        } else {
            "hover/hovpain2.wav"
        },
        2,
        1.0,
        1.0,
    );
    if !reacts_to_pain(context) {
        return;
    }
    let choice = context.game.random();
    context.set_move(
        if reaction.damage <= 25.0 {
            if choice < 0.5 {
                "hover_move_pain3"
            } else {
                "hover_move_pain2"
            }
        } else if choice < 0.3 {
            "hover_move_pain1"
        } else {
            "hover_move_pain2"
        },
        true,
    );
}

/// Die (`die`).
fn rerelease_hover_die(context: &mut MonsterContext, _reaction: &DeathReaction) {
    let actor = context.actor().clone();
    context.game.require_entity_mut(&actor).effects = 0;
    let owned = context.game.owned_of(actor.clone());
    context
        .game
        .host
        .combat()
        .set_powered_protection(&owned, &PoweredProtectionState::None);
    if check_gib(context) {
        hover_gib(context);
        return;
    }
    if context.state().dead {
        return;
    }
    let first = context.game.random() < 0.5;
    context.game.sound(
        &actor,
        if first {
            "hover/hovdeth1.wav"
        } else {
            "hover/hovdeth2.wav"
        },
        2,
        1.0,
        1.0,
    );
    context.state_mut().dead = true;
    context.state_mut().can_take_damage = true;
    let owned = context.game.owned_of(actor);
    context.game.set_combat_traits(
        &owned,
        &CombatTraitChanges {
            can_take_damage: Some(true),
            ..CombatTraitChanges::default()
        },
    );
    context.set_move("hover_move_death1", true);
}

/// Attack (`hover_attack`).
fn hover_attack(context: &mut MonsterContext) {
    if context.game.random() > 0.5 {
        context.state_mut().attack_state = MonsterAttackState::Straight;
        context.set_move("hover_move_attack1", true);
        return;
    }
    if context.game.random() <= 0.5 {
        let lefty = context.state().lefty;
        context.state_mut().lefty = !lefty;
    }
    context.state_mut().attack_state = MonsterAttackState::Sliding;
    context.set_move("hover_move_attack2", true);
}

/// Reattack (`hover_reattack`).
fn hover_reattack(context: &mut MonsterContext) {
    if alive_enemy(context) && visible(context, None) && context.game.random() <= 0.6 {
        if context.state().attack_state == MonsterAttackState::Straight {
            context.set_move("hover_move_attack1", true);
            return;
        }
        if context.state().attack_state == MonsterAttackState::Sliding {
            context.set_move("hover_move_attack2", true);
            return;
        }
    }
    context.set_move("hover_move_end_attack", true);
}

/// Fire blaster (`hover_fire_blaster`).
fn hover_fire_blaster(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let frame = context.game.require_entity(&actor).frame;
    let flash = if frame & 1 != 0 {
        rerelease_flash::HOVER_BLASTER_2
    } else {
        rerelease_flash::HOVER_BLASTER_1
    };
    let Some((start, direction)) = monster_shot(context, flash as usize, 0.0) else {
        return;
    };
    let effects = if frame % 4 == 0 { 64 } else { 0 };
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
    monster_flash(context, flash, start, direction);
}

/// Dead (`hover_dead`).
fn hover_dead(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.state_mut().corpse = true;
    let mut moved = context.game.body_of(actor.clone());
    moved.bounds.min = vec3(-16.0, -16.0, -24.0);
    moved.bounds.max = vec3(16.0, 16.0, -8.0);
    context.game.write_body(actor.clone(), &moved, true);
    context.game.set_motion_kind(actor.clone(), Q2MotionKind::Toss);
    let now = context.game.host.now();
    context.game.require_entity_mut(&actor).timestamp = now + 15.0;
    context.game.schedule(actor, 0.1, hover_dead_think);
}

/// Dying (`hover_dying`).
fn hover_dying(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    if context.game.body_of(actor.clone()).ground.is_some() {
        hover_dead_think(actor, &mut *context.game);
        return;
    }
    if rerelease_random(context).integer_max(2) != 0 {
        return;
    }
    let origin = context.game.body_of(actor.clone()).origin;
    context.game.host_emit(Q2PresentationEvent::Effect(Q2EffectEvent {
        effect: "q2:plain-explosion".to_string(),
        origin,
        direction: vec3(0.0, 0.0, 0.0),
        count: 1,
        color: 0,
    }));
    let organic = rerelease_random(context).integer_max(2) != 0;
    throw_gib(
        actor,
        &mut *context.game,
        if organic {
            "models/objects/gibs/sm_meat/tris.md2"
        } else {
            "models/objects/gibs/sm_metal/tris.md2"
        },
        120.0,
        Q2GibOptions {
            metallic: !organic,
            ..Q2GibOptions::default()
        },
    );
}

/// Create the rerelease hover definition (`createRereleaseHoverDefinition`).
pub fn create_rerelease_hover_definition() -> Q2MonsterDefinition {
    let mut definition = hover_definition();
    definition.moves = hover_moves();
    let mut source_callbacks = Q2CallbackDefinitions::default();
    source_callbacks.think.insert("hover_deadthink", hover_dead_think);
    definition.source_callbacks = Some(source_callbacks);
    definition.initialize = Some(MonsterHandler::Callback(rerelease_hover_initialize));
    definition.pain = Some(rerelease_hover_pain);
    definition.die = rerelease_hover_die;
    for (name, handler) in [
        ("hover_attack", MonsterHandler::Callback(hover_attack)),
        ("hover_reattack", MonsterHandler::Callback(hover_reattack)),
        ("hover_fire_blaster", MonsterHandler::Callback(hover_fire_blaster)),
        ("hover_dead", MonsterHandler::Callback(hover_dead)),
        ("hover_dying", MonsterHandler::Callback(hover_dying)),
    ] {
        definition.callbacks.insert(name.to_string(), handler);
    }
    definition
}
