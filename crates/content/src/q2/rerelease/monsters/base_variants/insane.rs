//! Rerelease insane (`src/content/q2/rerelease/monsters/base-variants/insane.ts`).
//!
//! ZeniMax Media, GPL-2.0-or-later.

use super::super::common::check_gib;
use super::super::tables::insane::{insane_frame, insane_moves};
use crate::q2::base::monsters::insane::insane_definition;
use crate::q2::foundation::monsters::ai::{corpse, health};
use crate::q2::foundation::monsters::gibs::{Q2GibOptions, throw_gib};
use crate::q2::foundation::monsters::types::{
    DeadThink, MonsterContext, MonsterHandler, MonsterLocomotion, Q2MonsterDefinition,
    record_at,
};
use crate::q2::support::contracts::{
    CombatTraitChanges, DeathReaction, PainReaction,
};

/// Crawling (`crawling`).
fn insane_crawling(frame: i32) -> bool {
    (frame >= insane_frame::CRAWL1 && frame <= insane_frame::CRAWL9)
        || (frame >= insane_frame::STAND99 && frame <= insane_frame::STAND160)
}

/// Run (`run`).
fn rerelease_insane_run(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let entity = context.game.require_entity(&actor);
    let (spawnflags, frame) = (entity.spawnflags, entity.frame);
    if spawnflags & 16 != 0 && frame == insane_frame::CR_PAIN10 {
        context.set_move("insane_move_down", true);
        return;
    }
    let crawl = spawnflags & 4 != 0
        || insane_crawling(frame)
        || (frame >= insane_frame::CR_PAIN2 && frame <= insane_frame::CR_PAIN10);
    let normal = context.game.random() <= 0.5;
    context.set_move(
        if crawl {
            "insane_move_runcrawl"
        } else if normal {
            "insane_move_run_normal"
        } else {
            "insane_move_run_insane"
        },
        true,
    );
}

/// Dead (`dead`).
fn insane_dead(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    if context.game.require_entity(&actor).spawnflags & 8 == 0 {
        corpse(context);
        return;
    }
    let entity = context.game.require_entity_mut(&actor);
    entity.flags |= 1;
    entity.server_flags |= 2;
    context.state_mut().corpse = true;
    context.game.link_actor(actor);
    context.schedule(0.1, DeadThink::MonsterDeadThink);
}

/// Vocalize (`vocalize`).
fn insane_vocalize(context: &mut MonsterContext, scream: bool) {
    let actor = context.actor().clone();
    if context.game.require_entity(&actor).spawnflags & 64 != 0
        || context.state().attack_finished >= context.game.host.now()
    {
        return;
    }
    let sample = if scream {
        *record_at(
            &[1, 2, 3, 4, 6, 8, 9, 10],
            (context.game.random() * 8.0).floor() as usize,
        )
    } else {
        7
    };
    context.game.sound(&actor, &format!("insane/insane{sample}.wav"), 2, 1.0, 2.0);
    let now = context.game.host.now();
    context.state_mut().attack_finished = now + 1.0 + context.game.random() * 2.0;
}

/// Moan (`insane_moan`).
fn insane_moan(context: &mut MonsterContext) {
    insane_vocalize(context, false);
}

/// Scream (`insane_scream`).
fn insane_scream(context: &mut MonsterContext) {
    insane_vocalize(context, true);
}

/// Shake (`insane_shake`).
fn insane_shake(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    if context.game.require_entity(&actor).spawnflags & 64 != 0 {
        return;
    }
    context.game.sound(&actor, "insane/insane5.wav", 2, 1.0, 2.0);
}

/// Initialize (`initialize`).
fn rerelease_insane_initialize(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.state_mut().good_guy = true;
    if context.game.require_entity(&actor).spawnflags & 16 != 0 {
        context.state_mut().stand_ground = true;
    }
    if context.game.require_entity(&actor).spawnflags & 8 != 0 {
        context.state_mut().locomotion = MonsterLocomotion::Stationary;
        context.game.require_entity_mut(&actor).flags |= 2048 | 262144;
    }
}

/// After spawn (`afterSpawn`).
fn rerelease_insane_after_spawn(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let skin = (context.game.random() * 3.0).floor() as i32;
    context.game.require_entity_mut(&actor).skin = skin;
}

/// Pain (`pain`).
fn rerelease_insane_pain(context: &mut MonsterContext, _reaction: &PainReaction) {
    let actor = context.actor().clone();
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    let variant = 1 + (context.game.random() * 2.0).floor() as i32;
    let hp = health(&mut *context.game, Some(&actor));
    let band = if hp < 25.0 {
        25
    } else if hp < 50.0 {
        50
    } else if hp < 75.0 {
        75
    } else {
        100
    };
    context.game.sound(
        &actor,
        &format!("player/male/pain{band}_{variant}.wav"),
        2,
        1.0,
        2.0,
    );
    let entity = context.game.require_entity(&actor);
    let (spawnflags, frame) = (entity.spawnflags, entity.frame);
    context.set_move(
        if spawnflags & 8 != 0 {
            "insane_move_struggle_cross"
        } else if insane_crawling(frame)
            || (frame >= insane_frame::STAND1 && frame <= insane_frame::STAND40)
        {
            "insane_move_crawl_pain"
        } else {
            "insane_move_stand_pain"
        },
        true,
    );
}

/// Die (`die`).
fn rerelease_insane_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    let actor = context.actor().clone();
    if check_gib(context) {
        context.game.sound(&actor, "misc/udeath.wav", 2, 1.0, 2.0);
        let damage = reaction.pain.damage;
        for _ in 0..2 {
            throw_gib(
                actor.clone(),
                &mut *context.game,
                "models/objects/gibs/bone/tris.md2",
                damage,
                Q2GibOptions::default(),
            );
        }
        for _ in 0..4 {
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
            "models/objects/gibs/head2/tris.md2",
            damage,
            Q2GibOptions {
                head: true,
                ..Q2GibOptions::default()
            },
        );
        context.state_mut().dead = true;
        context.state_mut().gibbed = true;
        return;
    }
    if context.state().dead {
        return;
    }
    let variant = 1 + (context.game.random() * 4.0).floor() as i32;
    context.game.sound(&actor, &format!("player/male/death{variant}.wav"), 2, 1.0, 2.0);
    context.state_mut().dead = true;
    context.state_mut().can_take_damage = true;
    let owned = context.game.owned_of(actor.clone());
    context.game.set_combat_traits(
        &owned,
        &CombatTraitChanges {
            can_take_damage: Some(true),
            ..CombatTraitChanges::default()
        },
    );
    if context.game.require_entity(&actor).spawnflags & 8 != 0 {
        insane_dead(context);
    } else {
        let frame = context.game.require_entity(&actor).frame;
        context.set_move(
            if insane_crawling(frame) {
                "insane_move_crawl_death"
            } else {
                "insane_move_stand_death"
            },
            true,
        );
    }
}

/// Create the rerelease insane definition (`rereleaseInsaneDefinition`).
pub fn rerelease_insane_definition() -> Q2MonsterDefinition {
    let mut definition = insane_definition();
    definition.moves = insane_moves();
    definition.run = MonsterHandler::Callback(rerelease_insane_run);
    definition.initialize = Some(MonsterHandler::Callback(rerelease_insane_initialize));
    definition.after_spawn = Some(MonsterHandler::Callback(rerelease_insane_after_spawn));
    definition.pain = Some(rerelease_insane_pain);
    definition.die = rerelease_insane_die;
    for (name, handler) in [
        ("insane_run", MonsterHandler::Callback(rerelease_insane_run)),
        ("insane_dead", MonsterHandler::Callback(insane_dead)),
        ("insane_shake", MonsterHandler::Callback(insane_shake)),
        ("insane_moan", MonsterHandler::Callback(insane_moan)),
        ("insane_scream", MonsterHandler::Callback(insane_scream)),
    ] {
        definition.callbacks.insert(name.to_string(), handler);
    }
    definition
}
