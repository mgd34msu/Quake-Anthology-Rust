//! Rerelease arachnid (`src/content/q2/rerelease/monsters/arachnid.ts`).
//!
//! ZeniMax Media, GPL-2.0-or-later.

use qa_core::math::{normalize3, sub3, vec3, Bounds};

use super::common::{check_gib, reacts_to_pain};
use super::tables::arachnid::{arachnid_frame, arachnid_moves};
use super::tables::flashes::rerelease_flash;
use crate::q2::base::monsters::common::{finish_corpse_default, move_handler, sound_handler};
use crate::q2::foundation::host::Q2Edition;
use crate::q2::foundation::monsters::ai::{enemy_body, enemy_eye, project_flash, target_distance};
use crate::q2::foundation::monsters::gibs::{throw_gib, Q2GibOptions};
use crate::q2::foundation::monsters::muzzle::muzzle_offset;
use crate::q2::foundation::monsters::types::{MonsterContext, MonsterHandler, Q2MonsterDefinition};
use crate::q2::rerelease::monsters::common::monster_flash;
use crate::q2::support::contracts::{DeathReaction, PainReaction};

/// Run (`run`).
fn arachnid_run(context: &mut MonsterContext) {
    let stand_ground = context.state().stand_ground;
    context.set_move(
        if stand_ground {
            "arachnid_move_stand"
        } else {
            "arachnid_move_run"
        },
        false,
    );
}

/// Attack (`attack`).
fn arachnid_attack(context: &mut MonsterContext) {
    let Some(enemy) = enemy_body(context) else {
        return;
    };
    let actor = context.actor().clone();
    if context.state().melee_time < context.game.host.now() && target_distance(context) < 80.0 {
        context.set_move("arachnid_melee", true);
        return;
    }
    let above = enemy.origin.z - context.game.body_of(actor).origin.z > 150.0;
    context.set_move(
        if above {
            "arachnid_attack_up1"
        } else {
            "arachnid_attack1"
        },
        false,
    );
}

/// Pain (`pain`).
fn arachnid_pain(context: &mut MonsterContext, _reaction: &PainReaction) {
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    let actor = context.actor().clone();
    context.game.sound(&actor, "arachnid/pain.wav", 2, 1.0, 1.0);
    if !reacts_to_pain(context) {
        return;
    }
    let first = context.game.random() < 0.5;
    context.set_move(
        if first {
            "arachnid_move_pain1"
        } else {
            "arachnid_move_pain2"
        },
        false,
    );
}

/// Die (`die`).
fn arachnid_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    let actor = context.actor().clone();
    if check_gib(context) {
        context.game.sound(&actor, "misc/udeath.wav", 2, 1.0, 1.0);
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
            actor,
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
    let actor = context.actor().clone();
    context.game.sound(&actor, "arachnid/death.wav", 2, 1.0, 1.0);
    context.state_mut().dead = true;
    context.state_mut().can_take_damage = true;
    let owned = context.game.owned_of(actor);
    context.game.set_combat_traits(
        &owned,
        &crate::q2::support::contracts::CombatTraitChanges {
            can_take_damage: Some(true),
            ..crate::q2::support::contracts::CombatTraitChanges::default()
        },
    );
    context.set_move("arachnid_move_death", true);
}

/// Footstep (`arachnid_footstep`).
fn arachnid_footstep(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "insane/insane11.wav", 4, 0.5, 2.0);
}

/// Melee hit (`arachnid_melee_hit`).
fn arachnid_melee_hit(context: &mut MonsterContext) {
    let fire_hit = context.weapons.fire_hit;
    let actor = context.actor().clone();
    if !fire_hit(actor, &mut *context.game, vec3(80.0, 0.0, 0.0), 15.0, 50.0) {
        let melee = context.game.host.now() + 1.0;
        context.state_mut().melee_time = melee;
    }
}

/// Charge rail (`arachnid_charge_rail`).
fn arachnid_charge_rail(context: &mut MonsterContext) {
    let Some(target) = enemy_eye(context) else {
        return;
    };
    context.entity_mut().pos1 = target;
    let actor = context.actor().clone();
    context.game.sound(&actor, "gladiator/railgun.wav", 1, 1.0, 1.0);
}

/// Rail (`arachnid_rail`).
fn arachnid_rail(context: &mut MonsterContext) {
    let frame = context.entity().frame;
    let flash = if frame == arachnid_frame::RAILS8 {
        rerelease_flash::ARACHNID_RAIL2
    } else if frame == arachnid_frame::RAILS_UP7 {
        rerelease_flash::ARACHNID_RAIL_UP1
    } else if frame == arachnid_frame::RAILS_UP11 {
        rerelease_flash::ARACHNID_RAIL_UP2
    } else {
        rerelease_flash::ARACHNID_RAIL1
    };
    let start = project_flash(context, muzzle_offset(Q2Edition::Rerelease, flash as usize), None);
    let pos1 = context.entity().pos1;
    let direction = normalize3(sub3(pos1, start));
    let fire_rail = context.weapons.fire_rail;
    let actor = context.actor().clone();
    fire_rail(actor, &mut *context.game, start, direction, 35.0, 100.0);
    monster_flash(context, flash, start, direction);
}

/// Arachnid definition (`arachnidDefinition`).
pub fn arachnid_definition() -> Q2MonsterDefinition {
    let mut definition = Q2MonsterDefinition::new(
        "monster_arachnid",
        "arachnid",
        "models/monsters/arachnid/tris.md2",
        1000.0,
        -200.0,
        450.0,
        Bounds {
            min: vec3(-48.0, -48.0, -20.0),
            max: vec3(48.0, 48.0, 48.0),
        },
        1.0,
        "arachnid_move_stand",
        arachnid_moves(),
        move_handler("arachnid_move_stand"),
        move_handler("arachnid_move_walk"),
        MonsterHandler::Callback(arachnid_run),
        MonsterHandler::Callback(arachnid_attack),
        arachnid_die,
    );
    definition.sight = Some(sound_handler("arachnid/sight.wav", 2, 1.0));
    definition.pain = Some(arachnid_pain);
    for (name, handler) in [
        ("arachnid_run", MonsterHandler::Callback(arachnid_run)),
        ("arachnid_dead", MonsterHandler::Callback(finish_corpse_default)),
        ("arachnid_footstep", MonsterHandler::Callback(arachnid_footstep)),
        ("arachnid_melee_charge", sound_handler("gladiator/melee3.wav", 1, 1.0)),
        ("arachnid_melee_hit", MonsterHandler::Callback(arachnid_melee_hit)),
        ("arachnid_charge_rail", MonsterHandler::Callback(arachnid_charge_rail)),
        ("arachnid_rail", MonsterHandler::Callback(arachnid_rail)),
    ] {
        definition.callbacks.insert(name.to_string(), handler);
    }
    definition
}
