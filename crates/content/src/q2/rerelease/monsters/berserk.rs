//! Rerelease berserk (`src/content/q2/rerelease/monsters/berserk.ts`).
//!
//! ZeniMax Media, GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::{Vec3, add3, length3, normalize3, scale3, sub3, vec3};

use super::common::{
    JumpNavigation, JumpResult, blocked_check_jump, blocked_check_platform,
    check_gib, monster_jump_finished, predicted_direction, reacts_to_pain,
};
use super::tables::berserk::{berserk_frame, berserk_moves};
use crate::q2::base::monsters::common::{HUMANOID_BOUNDS, move_handler, sound_handler};
use crate::q2::foundation::fields::number_field;
use crate::q2::foundation::host::{
    Q2EffectEvent, Q2PresentationEvent, Q2TraceRequest,
};
use crate::q2::foundation::monsters::ai::{
    angles_vectors, corpse, enemy_body, finish_dodge, health, project_flash,
    set_duck, target_distance, vector_angles,
};
use crate::q2::foundation::monsters::gibs::{Q2GibOptions, throw_gib};
use crate::q2::foundation::monsters::types::{
    MonsterAttackState, MonsterContext, MonsterHandler, Q2MonsterDefinition,
};
use crate::q2::support::contracts::{DeathReaction, PainReaction, TouchContact};

/// Jumping (`jumping`).
fn berserk_jumping(context: &mut MonsterContext) -> bool {
    let current = context.state().current_move.name.clone();
    current == "berserk_move_jump"
        || current == "berserk_move_jump2"
        || current == "berserk_move_attack_strike"
}

/// Run (`run`).
fn rerelease_berserk_run(context: &mut MonsterContext) {
    finish_dodge(context);
    let stand_ground = context.state().stand_ground;
    context.set_move(
        if stand_ground {
            "berserk_move_stand"
        } else {
            "berserk_move_run1"
        },
        false,
    );
}

/// Melee (`melee`).
fn rerelease_berserk_melee(context: &mut MonsterContext) {
    if context.state().melee_time > context.game.host.now() {
        return;
    }
    if context.state().current_move.name == "berserk_move_run_attack1"
        && context.entity().frame >= berserk_frame::R_ATT13
    {
        context.state_mut().attack_state = MonsterAttackState::Straight;
        context.state_mut().attack_finished = 0.0;
        return;
    }
    finish_dodge(context);
    let spike = context.game.random() < 0.5;
    context.set_move(
        if spike {
            "berserk_move_attack_spike"
        } else {
            "berserk_move_attack_club"
        },
        false,
    );
}

/// Slam radius damage (`slamRadiusDamage`).
pub fn slam_radius_damage(
    context: &mut MonsterContext,
    origin: Vec3,
    damage: f64,
    kick: f64,
    radius: f64,
) {
    let actor = context.actor().clone();
    let center = context.game.body_of(actor.clone()).origin;
    let nearby = context.game.host.nearby(center, radius * 2.0);
    let mut point = origin;
    for target in nearby {
        if &target == context.actor() {
            continue;
        }
        let damageable = context
            .game
            .host
            .combat()
            .read(&target)
            .is_some_and(|state| state.can_take_damage);
        if !damageable || !context.game.can_damage(&target, &actor) {
            continue;
        }
        let Some(target_body) = context.game.host.bodies().read(&target) else {
            continue;
        };
        if context.game.host.is_player(&target) && target_body.ground.is_none() {
            continue;
        }
        let min = add3(target_body.origin, target_body.bounds.min);
        let max = add3(target_body.origin, target_body.bounds.max);
        let closest = vec3(
            min.x.max(max.x.min(point.x)),
            min.y.max(max.y.min(point.y)),
            min.z.max(max.z.min(point.z)),
        );
        let amount = 1.0f64.min(1.0 - f64::from(length3(sub3(closest, point))) / radius);
        if amount <= 0.0 {
            continue;
        }
        let direction = normalize3(sub3(target_body.origin, point));
        point.z = min.z;
        context.game.damage(
            target.clone(),
            actor.clone(),
            Some(actor.clone()),
            (1.0f64.max(damage * amount * amount)).trunc(),
            (kick * amount * amount).trunc(),
            direction,
            point,
            direction,
            0,
            1,
            None,
        );
        let owned = context.game.host.actors().resolve_owned(&target);
        let after = context.game.host.bodies().read(&target);
        if context.game.host.is_player(&target) {
            if let (Some(owned), Some(after)) = (owned, after) {
                let mut moved = after;
                moved.velocity.z = 270.0f32.max(moved.velocity.z);
                context.game.host.bodies().write(&owned, &moved);
            }
        }
    }
}

/// Slam (`slam`).
fn berserk_slam(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "mutant/thud1.wav", 1, 1.0, 1.0);
    let actor = context.actor().clone();
    context.game.sound(&actor, "world/explod2.wav", 0, 0.75, 1.0);
    let start = project_flash(context, vec3(20.0, -14.3, -21.0), None);
    let actor = context.actor().clone();
    let origin = context.game.body_of(actor.clone()).origin;
    let trace = context.game.host.trace(&Q2TraceRequest {
        start: origin,
        end: start,
        bounds: None,
        ignore: Some(actor.clone()),
        mask: 3,
        exclude: Vec::new(),
    });
    context.game.host_emit(Q2PresentationEvent::Effect(Q2EffectEvent {
        effect: "q2:berserk-slam".to_string(),
        origin: trace.end,
        direction: vec3(0.0, 0.0, 1.0),
        count: 1,
        color: 0,
    }));
    context.entity_mut().gravity = 1.0;
    context.entity_mut().flags |= 1 << 23;
    let mut moved = context.game.body_of(actor.clone());
    moved.velocity = vec3(0.0, 0.0, 0.0);
    context.game.write_body(actor.clone(), &moved, true);
    let motion = context.game.require_entity(&actor).motion;
    context.game.set_motion_kind(actor, motion);
    slam_radius_damage(context, trace.end, 8.0, 300.0, 165.0);
}

/// High gravity (`highGravity`).
fn berserk_high_gravity(context: &mut MonsterContext) {
    let world = context.game.host.world_actor();
    let gravity = context
        .game
        .entity(&world)
        .map(|world| number_field(&world.spawn, "gravity", 800.0))
        .unwrap_or(800.0);
    let actor = context.actor().clone();
    let falling = context.game.body_of(actor.clone()).velocity.z < 0.0;
    context.entity_mut().gravity = (if falling { 2.25 } else { 5.25 }) * (800.0 / gravity);
    let motion = context.game.require_entity(&actor).motion;
    context.game.set_motion_kind(actor, motion);
}

/// Jump touch (`touch`).
fn berserk_jump_touch(actor: ActorId, game: &mut crate::q2::foundation::host::Q2GameServices, _contact: TouchContact) {
    if !game.monsters.states.contains_key(&actor) {
        return;
    }
    if health(game, Some(&actor)) <= 0.0 {
        game.require_entity_mut(&actor).touch = None;
        return;
    }
    if game.body_of(actor.clone()).ground.is_some() {
        game.require_entity_mut(&actor).frame = berserk_frame::SLAM18;
        if game.require_entity(&actor).touch.is_some() {
            let mut context = MonsterContext::new(actor.clone(), game);
            berserk_slam(&mut context);
            let game = &mut *context.game;
            game.require_entity_mut(&actor).touch = None;
        } else {
            game.require_entity_mut(&actor).touch = None;
        }
    }
}

/// Attack (`attack`).
fn rerelease_berserk_attack(context: &mut MonsterContext) {
    let distance = target_distance(context);
    if context.state().melee_time <= context.game.host.now() && distance < 80.0 {
        rerelease_berserk_melee(context);
        return;
    }
    if context.entity().spawnflags & 8 == 0
        && context.entity().timestamp < context.game.host.now()
        && context.game.random() < 0.5
        && distance > 150.0
    {
        context.set_move("berserk_move_attack_strike", false);
        let timestamp = context.game.host.now() + 5.0;
        context.entity_mut().timestamp = timestamp;
        let actor = context.actor().clone();
        context.game.sound(&actor, "berserk/jump.wav", 1, 1.0, 1.0);
        return;
    }
    if context.state().current_move.name == "berserk_move_run1" && distance <= 500.0 {
        context.set_move("berserk_move_run_attack1", false);
        let next = berserk_frame::R_ATT1 + context.entity().frame - berserk_frame::RUN1 + 1;
        context.state_mut().next_frame = next;
    }
}

/// Pain (`pain`).
fn rerelease_berserk_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    if berserk_jumping(context) || context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    let actor = context.actor().clone();
    context.game.sound(&actor, "berserk/berpain2.wav", 2, 1.0, 1.0);
    if !reacts_to_pain(context) {
        return;
    }
    finish_dodge(context);
    let light = reaction.damage <= 50.0 || context.game.random() < 0.5;
    context.set_move(
        if light {
            "berserk_move_pain1"
        } else {
            "berserk_move_pain2"
        },
        false,
    );
}

/// Die (`die`).
fn rerelease_berserk_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    let actor = context.actor().clone();
    if check_gib(context) {
        context.game.sound(&actor, "misc/udeath.wav", 2, 1.0, 1.0);
        context.entity_mut().skin = 0;
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
            "models/objects/gibs/gear/tris.md2",
            damage,
            Q2GibOptions::default(),
        );
        for part in ["chest", "hammer", "thigh", "head"] {
            let model = format!("models/monsters/berserk/gibs/{part}.md2");
            throw_gib(
                actor.clone(),
                &mut *context.game,
                &model,
                damage,
                Q2GibOptions {
                    skinned: true,
                    upright: part == "hammer",
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
    let actor = context.actor().clone();
    context.game.sound(&actor, "berserk/berdeth2.wav", 2, 1.0, 1.0);
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
    context.set_move(
        if reaction.pain.damage >= 50.0 {
            "berserk_move_death1"
        } else {
            "berserk_move_death2"
        },
        false,
    );
}

/// Duck (`duck`).
fn rerelease_berserk_duck(context: &mut MonsterContext, _eta: f64) -> bool {
    let current = context.state().current_move.name.clone();
    if context.game.random() >= 0.05
        || current == "berserk_move_jump"
        || current == "berserk_move_jump2"
    {
        return false;
    }
    context.set_move("berserk_move_duck2", false);
    true
}

/// Sidestep (`sidestep`).
fn rerelease_berserk_sidestep(context: &mut MonsterContext) -> bool {
    let current = context.state().current_move.name.clone();
    if berserk_jumping(context) || current == "berserk_move_pain2" {
        return false;
    }
    if current != "berserk_move_run1" {
        context.set_move("berserk_move_run1", false);
    }
    true
}

/// Blocked (`blocked`).
fn rerelease_berserk_blocked(context: &mut MonsterContext, distance: f64) -> bool {
    let can_jump = context.entity().spawnflags & 8 == 0;
    let jump = blocked_check_jump(context, distance, 256.0, 40.0, can_jump, JumpNavigation::None);
    if jump != JumpResult::None {
        if jump != JumpResult::Turn {
            context.set_move(
                if jump == JumpResult::Up {
                    "berserk_move_jump2"
                } else {
                    "berserk_move_jump"
                },
                false,
            );
        }
        return true;
    }
    blocked_check_platform(context, distance)
}

/// Fidget (`berserk_fidget`).
fn berserk_fidget(context: &mut MonsterContext) {
    if context.state().stand_ground
        || context.entity().enemy.is_some()
        || context.game.random() > 0.15
    {
        return;
    }
    context.set_move("berserk_move_stand_fidget", false);
    let actor = context.actor().clone();
    context.game.sound(&actor, "berserk/beridle1.wav", 1, 1.0, 2.0);
}

/// Attack spike (`berserk_attack_spike`).
fn berserk_attack_spike(context: &mut MonsterContext) {
    let fire_hit = context.weapons.fire_hit;
    let actor = context.actor().clone();
    let damage = 5.0 + (context.game.random() * 6.0).floor();
    if !fire_hit(actor, &mut *context.game, vec3(80.0, 0.0, -24.0), damage, 80.0) {
        let melee = context.game.host.now() + 1.2;
        context.state_mut().melee_time = melee;
    }
}

/// Attack club (`berserk_attack_club`).
fn berserk_attack_club(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let min_x = context.game.body_of(actor.clone()).bounds.min.x;
    let fire_hit = context.weapons.fire_hit;
    let damage = 15.0 + (context.game.random() * 6.0).floor();
    if !fire_hit(actor, &mut *context.game, vec3(80.0, min_x, -4.0), damage, 400.0) {
        let melee = context.game.host.now() + 2.5;
        context.state_mut().melee_time = melee;
    }
}

/// Run attack speed (`berserk_run_attack_speed`).
fn berserk_run_attack_speed(context: &mut MonsterContext) {
    if context.entity().enemy.is_some() && target_distance(context) < 80.0 {
        let next = context.entity().frame + 6;
        context.state_mut().next_frame = next;
        finish_dodge(context);
    }
}

/// Run swing (`berserk_run_swing`).
fn berserk_run_swing(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "berserk/attack.wav", 1, 1.0, 1.0);
    let melee = context.game.host.now() + 0.6;
    context.state_mut().melee_time = melee;
    if context.state().attack_state == MonsterAttackState::Sliding {
        finish_dodge(context);
    }
}

/// Jump takeoff (`berserk_jump_takeoff`).
fn berserk_jump_takeoff(context: &mut MonsterContext) {
    let Some(enemy) = enemy_body(context) else {
        return;
    };
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    let speed = f64::from(length3(sub3(body.origin, enemy.origin))) * 1.95;
    let Some(direction) = predicted_direction(context, body.origin, speed, false, 0.0) else {
        return;
    };
    let angles = vec3(body.angles.x, vector_angles(direction).y, body.angles.z);
    let forward = angles_vectors(angles).forward;
    let mut moved = body;
    moved.origin.z += 1.0;
    moved.velocity = vec3(
        scale3(forward, speed as f32).x,
        scale3(forward, speed as f32).y,
        450.0,
    );
    moved.angles = angles;
    moved.ground = None;
    context.game.write_body(actor, &moved, true);
    context.state_mut().ducked = true;
    let finished = context.game.host.now() + 3.0;
    context.state_mut().attack_finished = finished;
    context.entity_mut().touch = Some(berserk_jump_touch);
    berserk_high_gravity(context);
}

/// Check landing (`berserk_check_landing`).
fn berserk_check_landing(context: &mut MonsterContext) {
    berserk_high_gravity(context);
    let actor = context.actor().clone();
    if context.game.body_of(actor).ground.is_some() {
        context.state_mut().attack_finished = 0.0;
        set_duck(context, false);
        context.entity_mut().frame = berserk_frame::SLAM18;
        if context.entity().touch.is_some() {
            berserk_slam(context);
            context.entity_mut().touch = None;
        }
        context.entity_mut().flags &= !(1 << 23);
        return;
    }
    context.state_mut().next_frame = if context.game.host.now() > context.state().attack_finished {
        berserk_frame::SLAM3
    } else {
        berserk_frame::SLAM5
    };
}

/// Shrink (`berserk_shrink`).
fn berserk_shrink(context: &mut MonsterContext) {
    context.entity_mut().server_flags |= 2;
    let actor = context.actor().clone();
    let mut body = context.game.body_of(actor.clone());
    body.bounds.max.z = 0.0;
    context.game.write_body(actor, &body, true);
}

/// Jump now (`berserk_jump_now`).
fn berserk_jump_now(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    let axes = angles_vectors(body.angles);
    let mut moved = body;
    moved.velocity = add3(
        moved.velocity,
        add3(scale3(axes.forward, 100.0), scale3(axes.up, 300.0)),
    );
    context.game.write_body(actor, &moved, true);
}

/// Second jump now (`berserk_jump2_now`).
fn berserk_jump2_now(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    let axes = angles_vectors(body.angles);
    let mut moved = body;
    moved.velocity = add3(
        moved.velocity,
        add3(scale3(axes.forward, 150.0), scale3(axes.up, 400.0)),
    );
    context.game.write_body(actor, &moved, true);
}

/// Jump wait land (`berserk_jump_wait_land`).
fn berserk_jump_wait_land(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let landed = context.game.body_of(actor).ground.is_some() || monster_jump_finished(context);
    let next = context.entity().frame + if landed { 1 } else { 0 };
    context.state_mut().next_frame = next;
}

/// Create the rerelease berserk definition (`createRereleaseBerserkDefinition`).
pub fn create_rerelease_berserk_definition() -> Q2MonsterDefinition {
    let mut definition = Q2MonsterDefinition::new(
        "monster_berserk",
        "berserk",
        "models/monsters/berserk/tris.md2",
        240.0,
        -60.0,
        250.0,
        HUMANOID_BOUNDS,
        1.0,
        "berserk_move_stand",
        berserk_moves(),
        move_handler("berserk_move_stand"),
        move_handler("berserk_move_walk"),
        MonsterHandler::Callback(rerelease_berserk_run),
        MonsterHandler::Callback(rerelease_berserk_attack),
        rerelease_berserk_die,
    );
    definition.melee = Some(MonsterHandler::Callback(rerelease_berserk_melee));
    definition.sight = Some(sound_handler("berserk/sight.wav", 2, 1.0));
    definition.search = Some(sound_handler("berserk/bersrch1.wav", 2, 1.0));
    let mut source_callbacks = crate::q2::foundation::callbacks::Q2CallbackDefinitions::default();
    source_callbacks.touch.insert("berserk_jump_touch", berserk_jump_touch);
    definition.source_callbacks = Some(source_callbacks);
    definition.pain = Some(rerelease_berserk_pain);
    definition.duck = Some(rerelease_berserk_duck);
    definition.sidestep = Some(rerelease_berserk_sidestep);
    definition.blocked = Some(rerelease_berserk_blocked);
    for (name, handler) in [
        ("berserk_stand", move_handler("berserk_move_stand")),
        ("berserk_run", MonsterHandler::Callback(rerelease_berserk_run)),
        ("berserk_dead", MonsterHandler::Callback(corpse)),
        ("berserk_fidget", MonsterHandler::Callback(berserk_fidget)),
        ("berserk_swing", sound_handler("berserk/attack.wav", 1, 1.0)),
        ("berserk_attack_spike", MonsterHandler::Callback(berserk_attack_spike)),
        ("berserk_attack_club", MonsterHandler::Callback(berserk_attack_club)),
        ("berserk_run_attack_speed", MonsterHandler::Callback(berserk_run_attack_speed)),
        ("berserk_run_swing", MonsterHandler::Callback(berserk_run_swing)),
        ("berserk_high_gravity", MonsterHandler::Callback(berserk_high_gravity)),
        ("berserk_jump_takeoff", MonsterHandler::Callback(berserk_jump_takeoff)),
        ("berserk_check_landing", MonsterHandler::Callback(berserk_check_landing)),
        ("berserk_shrink", MonsterHandler::Callback(berserk_shrink)),
        ("berserk_jump_now", MonsterHandler::Callback(berserk_jump_now)),
        ("berserk_jump2_now", MonsterHandler::Callback(berserk_jump2_now)),
        ("berserk_jump_wait_land", MonsterHandler::Callback(berserk_jump_wait_land)),
    ] {
        definition.callbacks.insert(name.to_string(), handler);
    }
    definition
}
