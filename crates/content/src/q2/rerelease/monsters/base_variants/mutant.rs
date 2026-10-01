//! Rerelease mutant (`src/content/q2/rerelease/monsters/base-variants/mutant.ts`).
//!
//! ZeniMax Media, GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::{add3, length3, normalize3, scale3, sub3, vec3};

use super::super::common::{
    blocked_check_jump, blocked_check_platform, check_gib, monster_jump_finished, reacts_to_pain, JumpNavigation,
    JumpResult,
};
use super::super::tables::mutant::{mutant_frame, mutant_moves};
use crate::q2::base::monsters::common::alive_enemy;
use crate::q2::base::monsters::mutant::{create_mutant_definition, MutantSource};
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::host::Q2GameServices;
use crate::q2::foundation::monsters::ai::{
    angles_vectors, check_bottom, corpse, enemy_body, health, target_distance, walk_move,
};
use crate::q2::foundation::monsters::gibs::{throw_gib, Q2GibOptions};
use crate::q2::foundation::monsters::types::{MonsterAttackState, MonsterContext, MonsterHandler, Q2MonsterDefinition};
use crate::q2::support::contracts::{CombatTraitChanges, DeathReaction, PainReaction, TouchContact};

/// Hit (`hit`).
fn mutant_hit(context: &mut MonsterContext, right: bool) {
    let actor = context.actor().clone();
    let bounds = context.game.body_of(actor.clone()).bounds;
    let aim = vec3(80.0, if right { bounds.max.x } else { bounds.min.x }, 8.0);
    let damage = 5.0 + (context.game.random() * 10.0).floor();
    let fire_hit = context.weapons.fire_hit;
    let connected = fire_hit(actor.clone(), &mut *context.game, aim, damage, 100.0);
    if !connected {
        let now = context.game.host.now();
        context.state_mut().melee_time = now + 1.5;
    }
    context.game.sound(
        &actor,
        if connected {
            if right {
                "mutant/mutatck3.wav"
            } else {
                "mutant/mutatck2.wav"
            }
        } else {
            "mutant/mutatck1.wav"
        },
        1,
        1.0,
        1.0,
    );
}

/// Hit left (`mutant_hit_left`).
fn mutant_hit_left(context: &mut MonsterContext) {
    mutant_hit(context, false);
}

/// Hit right (`mutant_hit_right`).
fn mutant_hit_right(context: &mut MonsterContext) {
    mutant_hit(context, true);
}

/// Jump (`jump`).
fn mutant_jump(context: &mut MonsterContext, up: bool) {
    let actor = context.actor().clone();
    let mut moved = context.game.body_of(actor.clone());
    let axes = angles_vectors(moved.angles);
    moved.velocity = add3(
        moved.velocity,
        add3(
            scale3(axes.forward, if up { 200.0 } else { 100.0 }),
            scale3(axes.up, if up { 450.0 } else { 300.0 }),
        ),
    );
    context.game.write_body(actor, &moved, true);
}

/// Jump down (`mutant_jump_down`).
fn mutant_jump_down(context: &mut MonsterContext) {
    mutant_jump(context, false);
}

/// Jump up (`mutant_jump_up`).
fn mutant_jump_up(context: &mut MonsterContext) {
    mutant_jump(context, true);
}

/// Touch (`touch`).
fn mutant_touch(actor: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    if !game.monsters.states.contains_key(&actor) {
        return;
    }
    if health(game, Some(&actor)) <= 0.0 {
        game.require_entity_mut(&actor).touch = None;
        return;
    }
    let mut context = MonsterContext::new(actor.clone(), game);
    let body = context.game.body_of(actor.clone());
    let style = context.game.require_entity(&actor).style;
    let damageable = context
        .game
        .host
        .combat()
        .read(&contact.other)
        .is_some_and(|state| state.can_take_damage);
    if style == 1 && damageable && length3(body.velocity) > 30.0 {
        let normal = normalize3(body.velocity);
        let point = add3(body.origin, scale3(normal, body.bounds.max.x));
        let damage = (40.0 + context.game.random() * 10.0).trunc();
        let velocity = body.velocity;
        context.game.damage(
            contact.other,
            actor.clone(),
            Some(actor.clone()),
            damage,
            damage,
            velocity,
            point,
            normal,
            0,
            0,
            None,
        );
        context.game.require_entity_mut(&actor).style = 0;
    }
    let origin = context.game.body_of(actor.clone()).origin;
    if !check_bottom(&mut context, origin) {
        if context.game.body_of(actor.clone()).ground.is_some() {
            context.state_mut().next_frame = mutant_frame::ATTACK02;
            context.game.require_entity_mut(&actor).touch = None;
        }
        return;
    }
    context.game.require_entity_mut(&actor).touch = None;
}

/// Slide right (`ai_move_slide_right`).
fn mutant_slide_right(context: &mut MonsterContext, distance: f64) {
    let actor = context.actor().clone();
    let yaw = f64::from(context.game.body_of(actor).angles.y) + 90.0;
    walk_move(context, yaw, distance, true, true);
}

/// Slide left (`ai_move_slide_left`).
fn mutant_slide_left(context: &mut MonsterContext, distance: f64) {
    let actor = context.actor().clone();
    let yaw = f64::from(context.game.body_of(actor).angles.y) - 90.0;
    walk_move(context, yaw, distance, true, true);
}

/// Check attack (`checkAttack`).
fn rerelease_mutant_check_attack(context: &mut MonsterContext) -> bool {
    let actor = context.actor().clone();
    if !alive_enemy(context) {
        return false;
    }
    if target_distance(context) <= 80.0 && context.state().melee_time <= context.game.host.now() {
        context.state_mut().attack_state = MonsterAttackState::Melee;
        return true;
    }
    if context.game.require_entity(&actor).spawnflags & 8 != 0 {
        return false;
    }
    let Some(enemy) = enemy_body(context) else {
        return false;
    };
    let body = context.game.body_of(actor);
    if body.origin.z + body.bounds.min.z + 125.0 < enemy.origin.z + enemy.bounds.min.z {
        return false;
    }
    let delta = sub3(body.origin, enemy.origin);
    let distance = f64::hypot(f64::from(delta.x), f64::from(delta.y));
    if (distance < 100.0 && context.state().melee_time <= context.game.host.now())
        || distance > 265.0
        || context.state().attack_finished >= context.game.host.now()
        || context.game.random() >= 0.5
    {
        return false;
    }
    context.state_mut().attack_state = MonsterAttackState::Missile;
    true
}

/// Pain (`pain`).
fn rerelease_mutant_pain(context: &mut MonsterContext, _reaction: &PainReaction) {
    let actor = context.actor().clone();
    let max_health = context.game.require_entity(&actor).max_health;
    let bloodied = health(&mut *context.game, Some(&actor)) < max_health / 2.0;
    context.game.require_entity_mut(&actor).skin = if bloodied { 1 } else { 0 };
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    let choice = context.game.random();
    context.game.sound(
        &actor,
        if choice < 0.33 || choice >= 0.66 {
            "mutant/mutpain1.wav"
        } else {
            "mutant/mutpain2.wav"
        },
        2,
        1.0,
        1.0,
    );
    if !reacts_to_pain(context) {
        return;
    }
    context.set_move(
        if choice < 0.33 {
            "mutant_move_pain1"
        } else if choice < 0.66 {
            "mutant_move_pain2"
        } else {
            "mutant_move_pain3"
        },
        true,
    );
}

/// Die (`die`).
fn rerelease_mutant_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    let actor = context.actor().clone();
    if check_gib(context) {
        context.game.sound(&actor, "misc/udeath.wav", 2, 1.0, 1.0);
        context.game.require_entity_mut(&actor).skin /= 2;
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
        for _ in 0..2 {
            throw_gib(
                actor.clone(),
                &mut *context.game,
                "models/monsters/mutant/gibs/hand.md2",
                damage,
                Q2GibOptions {
                    skinned: true,
                    upright: true,
                    ..Q2GibOptions::default()
                },
            );
        }
        for _ in 0..2 {
            throw_gib(
                actor.clone(),
                &mut *context.game,
                "models/monsters/mutant/gibs/foot.md2",
                damage,
                Q2GibOptions {
                    skinned: true,
                    ..Q2GibOptions::default()
                },
            );
        }
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/monsters/mutant/gibs/chest.md2",
            damage,
            Q2GibOptions {
                skinned: true,
                ..Q2GibOptions::default()
            },
        );
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/monsters/mutant/gibs/head.md2",
            damage,
            Q2GibOptions {
                skinned: true,
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
    context.game.sound(&actor, "mutant/mutdeth1.wav", 2, 1.0, 1.0);
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
    let first = context.game.random() < 0.5;
    context.set_move(
        if first {
            "mutant_move_death1"
        } else {
            "mutant_move_death2"
        },
        true,
    );
}

/// Blocked (`blocked`).
fn rerelease_mutant_blocked(context: &mut MonsterContext, distance: f64) -> bool {
    let actor = context.actor().clone();
    let can_jump = context.game.require_entity(&actor).spawnflags & 8 == 0;
    let result = blocked_check_jump(context, distance, 256.0, 68.0, can_jump, JumpNavigation::None);
    if result != JumpResult::None {
        if result != JumpResult::Turn && context.entity().enemy.is_some() {
            context.set_move(
                if result == JumpResult::Up {
                    "mutant_move_jump_up"
                } else {
                    "mutant_move_jump_down"
                },
                true,
            );
        }
        return true;
    }
    blocked_check_platform(context, distance)
}

/// Step (`mutant_step`).
fn mutant_step(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let variant = (context.game.random() * 3.0).floor() as i32 + 1;
    context
        .game
        .sound(&actor, &format!("mutant/step{variant}.wav"), 4, 1.0, 1.0);
}

/// Check refire (`mutant_check_refire`).
fn mutant_check_refire(context: &mut MonsterContext) {
    if alive_enemy(context)
        && context.state().melee_time <= context.game.host.now()
        && (context.game.random() < 0.5 || target_distance(context) <= 80.0)
    {
        context.state_mut().next_frame = mutant_frame::ATTACK09;
    }
}

/// Jump takeoff (`mutant_jump_takeoff`).
fn mutant_jump_takeoff(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    let forward = angles_vectors(body.angles).forward;
    context.game.sound(&actor, "mutant/mutsght1.wav", 2, 1.0, 1.0);
    let mut moved = body;
    moved.origin.z += 1.0;
    let flat = scale3(forward, 425.0);
    moved.velocity = vec3(flat.x, flat.y, 160.0);
    moved.ground = None;
    context.game.write_body(actor.clone(), &moved, true);
    context.state_mut().ducked = true;
    let now = context.game.host.now();
    context.state_mut().attack_finished = now + 3.0;
    let entity = context.game.require_entity_mut(&actor);
    entity.style = 1;
    entity.touch = Some(mutant_touch);
}

/// Check landing (`mutant_check_landing`).
fn mutant_check_landing(context: &mut MonsterContext) {
    monster_jump_finished(context);
    let actor = context.actor().clone();
    if context.game.body_of(actor).ground.is_some() {
        let actor = context.actor().clone();
        context.game.sound(&actor, "mutant/thud1.wav", 1, 1.0, 1.0);
        let now = context.game.host.now();
        context.state_mut().attack_finished = now + 0.5 + context.game.random();
        if target_distance(context) <= 160.0 {
            context.melee();
        }
        return;
    }
    let expired = context.game.host.now() > context.state().attack_finished;
    context.state_mut().next_frame = if expired {
        mutant_frame::ATTACK02
    } else {
        mutant_frame::ATTACK05
    };
}

/// Jump wait land (`mutant_jump_wait_land`).
fn mutant_jump_wait_land(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let frame = context.game.require_entity(&actor).frame;
    let waiting = !monster_jump_finished(context) && context.game.body_of(actor).ground.is_none();
    context.state_mut().next_frame = if waiting { frame } else { frame + 1 };
}

/// Shrink (`mutant_shrink`).
fn mutant_shrink(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.require_entity_mut(&actor).server_flags |= 2;
    let mut moved = context.game.body_of(actor.clone());
    moved.bounds.max.z = 0.0;
    context.game.write_body(actor, &moved, true);
}

/// Monster dead (`monster_dead`).
fn mutant_monster_dead(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let bounds = context.game.body_of(actor.clone()).bounds;
    corpse(context);
    let mut moved = context.game.body_of(actor.clone());
    moved.bounds = bounds;
    context.game.write_body(actor, &moved, true);
}

/// Create the rerelease mutant definition (`createRereleaseMutantDefinition`).
pub fn create_rerelease_mutant_definition() -> Q2MonsterDefinition {
    let mut definition = create_mutant_definition(MutantSource::Classic);
    definition.moves = mutant_moves();
    let mut source_callbacks = Q2CallbackDefinitions::default();
    source_callbacks.touch.insert("mutant_jump_touch", mutant_touch);
    definition.source_callbacks = Some(source_callbacks);
    definition
        .ai
        .insert("ai_move_slide_right".to_string(), mutant_slide_right);
    definition
        .ai
        .insert("ai_move_slide_left".to_string(), mutant_slide_left);
    definition.check_attack = Some(rerelease_mutant_check_attack);
    definition.pain = Some(rerelease_mutant_pain);
    definition.die = rerelease_mutant_die;
    definition.blocked = Some(rerelease_mutant_blocked);
    for (name, handler) in [
        ("mutant_step", MonsterHandler::Callback(mutant_step)),
        ("mutant_hit_left", MonsterHandler::Callback(mutant_hit_left)),
        ("mutant_hit_right", MonsterHandler::Callback(mutant_hit_right)),
        ("mutant_check_refire", MonsterHandler::Callback(mutant_check_refire)),
        ("mutant_jump_takeoff", MonsterHandler::Callback(mutant_jump_takeoff)),
        ("mutant_check_landing", MonsterHandler::Callback(mutant_check_landing)),
        ("mutant_jump_down", MonsterHandler::Callback(mutant_jump_down)),
        ("mutant_jump_up", MonsterHandler::Callback(mutant_jump_up)),
        ("mutant_jump_wait_land", MonsterHandler::Callback(mutant_jump_wait_land)),
        ("mutant_shrink", MonsterHandler::Callback(mutant_shrink)),
        ("monster_dead", MonsterHandler::Callback(mutant_monster_dead)),
    ] {
        definition.callbacks.insert(name.to_string(), handler);
    }
    definition
}
