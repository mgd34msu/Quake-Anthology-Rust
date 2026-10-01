//! Mutant monster (`src/content/q2/base/monsters/mutant.ts`).
//!
//! Quake II m_mutant.c. id Software, GPL-2.0-or-later.

use std::collections::HashMap;

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3, add3, length3, normalize3, scale3, sub3, vec3};

use super::common::{
    alive_enemy, begin_death, damaged_skin, finish_corpse_default, move_handler, sound_handler,
    standard_gib,
};
use super::tables::mutant::{mutant_frame, mutant_moves};
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::host::{Q2GameServices, Q2Touch};
use crate::q2::foundation::monsters::ai::{
    angles_vectors, check_bottom, enemy_body, fly_check, health, target_distance,
};
use crate::q2::foundation::monsters::types::{
    MonsterAttackState, MonsterContext, MonsterHandler, Q2MonsterDefinition,
};
use crate::q2::support::contracts::{DeathReaction, PainReaction, TouchContact};

/// Mutant touch source (`source`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MutantSource {
    /// Classic tables.
    Classic,
    /// Rogue tables.
    Rogue,
}

/// Run (`run`).
fn mutant_run(context: &mut MonsterContext) {
    if context.state().stand_ground {
        context.set_move("mutant_move_stand", true);
    } else {
        context.set_move("mutant_move_run", true);
    }
}

/// Hit (`hit`).
fn mutant_hit(context: &mut MonsterContext, right: bool) {
    let fire_hit = context.weapons.fire_hit;
    let actor = context.actor().clone();
    let bounds = context.game.body_of(actor.clone()).bounds;
    let damage = 10.0 + (context.game.random() * 5.0).floor();
    let connected = fire_hit(
        actor.clone(),
        &mut *context.game,
        vec3(80.0, if right { bounds.max.x } else { bounds.min.x }, 8.0),
        damage,
        100.0,
    );
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

/// Shared jump-touch continuation (`touch`).
fn mutant_jump_touch_impl(actor: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    if !game.monsters.states.contains_key(&actor) {
        panic!("Mutant jump has no source continuation");
    }
    if health(game, Some(&actor)) <= 0.0 {
        game.require_entity_mut(&actor).touch = None;
        return;
    }
    let body = game.body_of(actor.clone());
    let damageable = game
        .host
        .combat()
        .read(&contact.other)
        .is_some_and(|state| state.can_take_damage);
    if damageable && length3(body.velocity) > 400.0 {
        let normal = normalize3(body.velocity);
        let point = add3(body.origin, scale3(normal, body.bounds.max.x));
        let damage = (40.0 + 10.0 * game.random()).trunc();
        game.damage(
            contact.other.clone(),
            actor.clone(),
            Some(actor.clone()),
            damage,
            damage,
            body.velocity,
            point,
            normal,
            0,
            0,
            None,
        );
    }
    let mut context = MonsterContext::new(actor.clone(), game);
    let origin = context.game.body_of(actor.clone()).origin;
    if !check_bottom(&mut context, origin) {
        if context.game.body_of(actor.clone()).ground.is_some() {
            context.state_mut().next_frame = mutant_frame::ATTACK02;
            context.entity_mut().touch = None;
        }
        return;
    }
    context.entity_mut().touch = None;
}

/// Classic jump touch (`classic_mutant_jump_touch`).
pub fn classic_mutant_jump_touch(
    actor: ActorId,
    game: &mut crate::q2::foundation::host::Q2GameServices,
    contact: TouchContact,
) {
    mutant_jump_touch_impl(actor, game, contact);
}

/// Rogue jump touch (`rogue_mutant_jump_touch`).
pub fn rogue_mutant_jump_touch(
    actor: ActorId,
    game: &mut crate::q2::foundation::host::Q2GameServices,
    contact: TouchContact,
) {
    mutant_jump_touch_impl(actor, game, contact);
}

/// Mutant source callbacks for a touch source.
pub fn mutant_source_callbacks(source: MutantSource) -> Q2CallbackDefinitions {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks.touch.insert(
        match source {
            MutantSource::Classic => "classic_mutant_jump_touch",
            MutantSource::Rogue => "rogue_mutant_jump_touch",
        },
        match source {
            MutantSource::Classic => classic_mutant_jump_touch as Q2Touch,
            MutantSource::Rogue => rogue_mutant_jump_touch as Q2Touch,
        },
    );
    callbacks
}

/// Idle (`idle`).
fn mutant_idle(context: &mut MonsterContext) {
    context.set_move("mutant_move_idle", true);
    let actor = context.actor().clone();
    context.game.sound(&actor, "mutant/mutidle1.wav", 2, 1.0, 2.0);
}

/// Check attack (`checkAttack`).
fn mutant_check_attack(context: &mut MonsterContext) -> bool {
    if !alive_enemy(context) {
        return false;
    }
    if target_distance(context) < 80.0 {
        context.state_mut().attack_state = MonsterAttackState::Melee;
        return true;
    }
    let Some(enemy) = enemy_body(context) else {
        return false;
    };
    let actor = context.actor().clone();
    let body = context.game.body_of(actor);
    let enemy_minimum = f64::from(enemy.origin.z) + f64::from(enemy.bounds.min.z);
    let enemy_size = f64::from(enemy.bounds.max.z) - f64::from(enemy.bounds.min.z);
    if f64::from(body.origin.z) + f64::from(body.bounds.min.z) > enemy_minimum + 0.75 * enemy_size
        || f64::from(body.origin.z) + f64::from(body.bounds.max.z)
            < enemy_minimum + 0.25 * enemy_size
    {
        return false;
    }
    let delta = sub3(body.origin, enemy.origin);
    let distance = f64::from(delta.x).hypot(f64::from(delta.y));
    if distance < 100.0 || distance > 100.0 && context.game.random() < 0.9 {
        return false;
    }
    context.state_mut().attack_state = MonsterAttackState::Missile;
    true
}

/// Pain (`pain`).
fn mutant_pain(context: &mut MonsterContext, _reaction: &PainReaction) {
    damaged_skin(context);
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    if context.game.options.skill == 3 {
        return;
    }
    let r = context.game.random();
    let actor = context.actor().clone();
    context.game.sound(
        &actor,
        if r < 0.33 || r >= 0.66 {
            "mutant/mutpain1.wav"
        } else {
            "mutant/mutpain2.wav"
        },
        2,
        1.0,
        1.0,
    );
    context.set_move(
        if r < 0.33 {
            "mutant_move_pain1"
        } else if r < 0.66 {
            "mutant_move_pain2"
        } else {
            "mutant_move_pain3"
        },
        false,
    );
}

/// Die (`die`).
fn mutant_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    if standard_gib(
        context,
        reaction,
        2,
        4,
        "models/objects/gibs/head2/tris.md2",
        1.0,
    ) || context.state().dead
    {
        return;
    }
    context.entity_mut().skin = 1;
    let first = context.game.random() < 0.5;
    begin_death(
        context,
        reaction,
        "mutant/mutdeth1.wav",
        if first {
            "mutant_move_death1"
        } else {
            "mutant_move_death2"
        },
        2,
        4,
    );
}

/// Idle loop (`mutant_idle_loop`).
fn mutant_idle_loop(context: &mut MonsterContext) {
    if context.game.random() < 0.75 {
        context.state_mut().next_frame = mutant_frame::STAND155;
    }
}

/// Step (`mutant_step`).
fn mutant_step(context: &mut MonsterContext) {
    let n = ((context.game.random() * 3.0).floor() + 1.0) as i32 % 3;
    let actor = context.actor().clone();
    context.game.sound(&actor, &format!("mutant/step{}.wav", n + 1), 2, 1.0, 1.0);
}

/// Hit left (`mutant_hit_left`).
fn mutant_hit_left(context: &mut MonsterContext) {
    mutant_hit(context, false);
}

/// Hit right (`mutant_hit_right`).
fn mutant_hit_right(context: &mut MonsterContext) {
    mutant_hit(context, true);
}

/// Check refire (`mutant_check_refire`).
fn mutant_check_refire(context: &mut MonsterContext) {
    if alive_enemy(context)
        && (context.game.options.skill == 3 && context.game.random() < 0.5
            || target_distance(context) < 80.0)
    {
        context.state_mut().next_frame = mutant_frame::ATTACK09;
    }
}

/// Jump takeoff (`mutant_jump_takeoff`).
fn mutant_jump_takeoff(context: &mut MonsterContext, touch: Q2Touch) {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    let forward = angles_vectors(body.angles).forward;
    context.game.sound(&actor, "mutant/mutsght1.wav", 2, 1.0, 1.0);
    let flat = scale3(forward, 600.0);
    let mut moved = body;
    moved.origin.z += 1.0;
    moved.velocity = vec3(flat.x, flat.y, 250.0);
    moved.ground = None;
    context.game.write_body(actor, &moved, true);
    context.state_mut().ducked = true;
    let now = context.game.host.now();
    context.state_mut().attack_finished = now + 3.0;
    context.entity_mut().touch = Some(touch);
}

/// Classic jump takeoff (`mutant_jump_takeoff`).
fn mutant_jump_takeoff_classic(context: &mut MonsterContext) {
    mutant_jump_takeoff(context, classic_mutant_jump_touch);
}

/// Rogue jump takeoff (`mutant_jump_takeoff`).
fn mutant_jump_takeoff_rogue(context: &mut MonsterContext) {
    mutant_jump_takeoff(context, rogue_mutant_jump_touch);
}

/// Check landing (`mutant_check_landing`).
fn mutant_check_landing(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    if context.game.body_of(actor.clone()).ground.is_some() {
        context.game.sound(&actor, "mutant/thud1.wav", 1, 1.0, 1.0);
        context.state_mut().attack_finished = 0.0;
        context.state_mut().ducked = false;
        return;
    }
    let expired = context.game.host.now() > context.state().attack_finished;
    context.state_mut().next_frame = if expired {
        mutant_frame::ATTACK02
    } else {
        mutant_frame::ATTACK05
    };
}

/// Dead (`mutant_dead`).
fn mutant_dead(context: &mut MonsterContext) {
    finish_corpse_default(context);
    fly_check(context);
}

/// Create a mutant definition (`createMutantDefinition`).
pub fn create_mutant_definition(source: MutantSource) -> Q2MonsterDefinition {
    let mut definition = Q2MonsterDefinition::new(
        "monster_mutant",
        "mutant",
        "models/monsters/mutant/tris.md2",
        300.0,
        -120.0,
        300.0,
        Bounds {
            min: Vec3 {
                x: -32.0,
                y: -32.0,
                z: -24.0,
            },
            max: Vec3 {
                x: 32.0,
                y: 32.0,
                z: 48.0,
            },
        },
        1.0,
        "mutant_move_stand",
        mutant_moves(),
        move_handler("mutant_move_stand"),
        move_handler("mutant_move_start_walk"),
        MonsterHandler::Callback(mutant_run),
        move_handler("mutant_move_jump"),
        mutant_die,
    );
    definition.melee = Some(move_handler("mutant_move_attack"));
    definition.sight = Some(sound_handler("mutant/mutsght1.wav", 2, 1.0));
    definition.search = Some(sound_handler("mutant/mutsrch1.wav", 2, 1.0));
    definition.idle = Some(MonsterHandler::Callback(mutant_idle));
    definition.pain = Some(mutant_pain);
    definition.check_attack = Some(mutant_check_attack);
    definition.source_callbacks = Some(mutant_source_callbacks(source));
    let takeoff = match source {
        MutantSource::Classic => MonsterHandler::Callback(mutant_jump_takeoff_classic),
        MutantSource::Rogue => MonsterHandler::Callback(mutant_jump_takeoff_rogue),
    };
    definition.callbacks = HashMap::from([
        (
            "mutant_stand".to_string(),
            move_handler("mutant_move_stand"),
        ),
        (
            "mutant_walk".to_string(),
            move_handler("mutant_move_start_walk"),
        ),
        (
            "mutant_walk_loop".to_string(),
            move_handler("mutant_move_walk"),
        ),
        (
            "mutant_run".to_string(),
            MonsterHandler::Callback(mutant_run),
        ),
        (
            "mutant_idle_loop".to_string(),
            MonsterHandler::Callback(mutant_idle_loop),
        ),
        (
            "mutant_step".to_string(),
            MonsterHandler::Callback(mutant_step),
        ),
        (
            "mutant_hit_left".to_string(),
            MonsterHandler::Callback(mutant_hit_left),
        ),
        (
            "mutant_hit_right".to_string(),
            MonsterHandler::Callback(mutant_hit_right),
        ),
        (
            "mutant_check_refire".to_string(),
            MonsterHandler::Callback(mutant_check_refire),
        ),
        ("mutant_jump_takeoff".to_string(), takeoff),
        (
            "mutant_check_landing".to_string(),
            MonsterHandler::Callback(mutant_check_landing),
        ),
        (
            "mutant_dead".to_string(),
            MonsterHandler::Callback(mutant_dead),
        ),
    ]);
    definition
}
