//! Xatrix fixbot (`src/content/q2/missionpacks/monsters/fixbot.ts`).
//!
//! Quake II xatrix/m_fixbot.c. ZeniMax Media, GPL-2.0-or-later.

use std::collections::BTreeMap;

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, add3, length3, scale3, sub3, vec3};

use super::dabeam::monster_dabeam;
use super::tables::xatrix_fixbot::{fixbot_frame, fixbot_moves};
use crate::q2::base::monsters::common::{
    finish_corpse_default, monster_explode, monster_muzzle, monster_shot,
    move_handler,
};
use crate::q2::foundation::callbacks::{Q2CallbackDefinitions, free_q2_entity};
use crate::q2::foundation::host::{
    Q2EffectEvent, Q2Entity, Q2PresentationEvent, Q2Solid, Q2TraceRequest,
};
use crate::q2::foundation::monsters::ai::{
    MASK_SHOT, angles_vectors, change_yaw, health, in_front, monster_solid_mask,
    project_flash, run_ai, vector_angles, visible,
};
use crate::q2::foundation::monsters::perception::found_target;
use crate::q2::foundation::monsters::types::{
    MonsterAi, MonsterContext, MonsterHandler, MonsterLocomotion,
    Q2MonsterDefinition,
};
use crate::q2::foundation::monsters::respawn_monster;
use crate::q2::foundation::weapons::types::Mod;
use crate::q2::support::contracts::{DeathReaction, PainReaction, TraceHit};

/// Run (`run`).
fn fixbot_run(context: &mut MonsterContext) {
    context.set_move(
        if context.state().stand_ground {
            "fixbot_move_stand"
        } else {
            "fixbot_move_run"
        },
        false,
    );
}

/// Goal (`goal`).
fn fixbot_goal(context: &mut MonsterContext) -> Q2Entity {
    let goal = context.entity().goal.clone();
    goal.as_ref()
        .and_then(|goal| context.game.entity(goal))
        .cloned()
        .unwrap_or_else(|| panic!("Fixbot source movement requires its bot goal"))
}

/// Turn (`turn`).
fn fixbot_turn(context: &mut MonsterContext) -> f64 {
    let goal = fixbot_goal(context);
    let actor = context.actor().clone();
    let goal_body = context.game.body_of(goal.actor.id().clone());
    let body = context.game.body_of(actor);
    let direction = sub3(goal_body.origin, body.origin);
    context.state_mut().ideal_yaw = f64::from(vector_angles(direction).y);
    change_yaw(context);
    f64::from(length3(direction).trunc())
}

/// Leave goal (`leaveGoal`).
fn fixbot_leave_goal(context: &mut MonsterContext) {
    let target = fixbot_goal(context);
    let target_id = target.actor.id().clone();
    context.game.schedule(target_id, 0.1, free_q2_entity);
    context.entity_mut().goal = None;
    context.entity_mut().enemy = None;
    context.set_move("fixbot_move_stand", false);
}

/// Vertical goal (`verticalGoal`).
fn fixbot_vertical_goal(context: &mut MonsterContext, landing: bool) {
    let actor = context.actor().clone();
    let target = context.game.create("bot_goal", BTreeMap::new());
    let body = context.game.body_of(actor.clone());
    context.game.require_entity_mut(&target).owner = Some(actor.clone());
    context.game.set_solid(target.clone(), Q2Solid::Box);
    let bounds = Bounds {
        min: vec3(-32.0, -32.0, -24.0),
        max: vec3(32.0, 32.0, 24.0),
    };
    let end = add3(
        body.origin,
        scale3(
            angles_vectors(body.angles).up,
            if landing { -8096.0 } else { 128.0 },
        ),
    );
    let trace = context.game.host.trace(&Q2TraceRequest {
        start: body.origin,
        end,
        bounds: Some(bounds),
        ignore: Some(actor.clone()),
        mask: monster_solid_mask(&*context.game),
        exclude: Vec::new(),
    });
    let mut moved = context.game.body_of(target.clone());
    moved.origin = trace.end;
    moved.bounds = bounds;
    context.game.write_body(target.clone(), &moved, false);
    context.entity_mut().goal = Some(target.clone());
    context.entity_mut().enemy = Some(target);
    context.set_move(
        if landing {
            "fixbot_move_landing"
        } else {
            "fixbot_move_takeoff"
        },
        false,
    );
}

/// Search (`search`).
fn fixbot_search(context: &mut MonsterContext) -> bool {
    if context.entity().goal.is_some() {
        return false;
    }
    let actor = context.actor().clone();
    let origin = context.game.body_of(actor.clone()).origin;
    let nearby = context.game.host.nearby(origin, 1024.0);
    let mut best: Option<ActorId> = None;
    let mut best_health = 0.0;
    for candidate_id in nearby {
        let candidate = context.game.entity(&candidate_id).cloned();
        let Some(candidate) = candidate else {
            continue;
        };
        if candidate.actor.id() == &actor
            || candidate.server_flags & 4 == 0
            || candidate.owner.is_some()
            || health(&mut *context.game, Some(&candidate_id)) > 0.0
            || candidate.next_think.is_some()
            || !visible(context, Some(&candidate_id))
        {
            continue;
        }
        let good_guy = context
            .game
            .monsters
            .states
            .get(&candidate_id)
            .is_some_and(|state| state.good_guy);
        if good_guy {
            continue;
        }
        if best.is_none() || candidate.max_health > best_health {
            best_health = candidate.max_health;
            best = Some(candidate_id);
        }
    }
    let Some(best) = best else {
        return false;
    };
    let enemy = context.entity().enemy.clone();
    context.state_mut().old_enemy = enemy;
    context.entity_mut().enemy = Some(best.clone());
    context.game.require_entity_mut(&best).owner = Some(actor);
    context.state_mut().medic = true;
    found_target(context);
    true
}

/// Attack (`attack`).
fn fixbot_attack(context: &mut MonsterContext) {
    if context.state().medic {
        let goal = context.entity().goal.clone();
        if !visible(context, goal.as_ref()) {
            return;
        }
        let enemy = context.entity().enemy.clone();
        let Some(enemy_id) = enemy else {
            return;
        };
        let Some(enemy_body) = context.game.host.bodies().read(&enemy_id) else {
            return;
        };
        let actor = context.actor().clone();
        let origin = context.game.body_of(actor).origin;
        if length3(sub3(origin, enemy_body.origin)) > 128.0 {
            return;
        }
        context.set_move("fixbot_move_laserattack", false);
        return;
    }
    context.set_move("fixbot_move_attack2", false);
}

/// Walk (`walk`).
fn fixbot_walk(context: &mut MonsterContext) {
    let goal = context.entity().goal.clone();
    let target = goal
        .as_ref()
        .and_then(|goal| context.game.entity(goal))
        .cloned();
    let repair = match target {
        Some(target) if target.classname == "object_repair" => {
            let actor = context.actor().clone();
            let origin = context.game.body_of(actor).origin;
            let target_body = context.game.body_of(target.actor.id().clone());
            length3(sub3(origin, target_body.origin)) < 32.0
        }
        _ => false,
    };
    context.set_move(
        if repair {
            "fixbot_move_weld_start"
        } else {
            "fixbot_move_walk"
        },
        false,
    );
}

/// Pain (`pain`).
fn fixbot_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    let actor = context.actor().clone();
    context.game.sound(&actor, "flyer/flypain1.wav", 2, 1.0, 1.0);
    context.set_move(
        if reaction.damage <= 10.0 {
            "fixbot_move_pain3"
        } else if reaction.damage <= 25.0 {
            "fixbot_move_painb"
        } else {
            "fixbot_move_paina"
        },
        false,
    );
}

/// Die (`die`).
fn fixbot_die(context: &mut MonsterContext, _reaction: &DeathReaction) {
    monster_explode(context, "flyer/flydeth1.wav");
}

/// Scripted move AI (`ai_move2`).
fn fixbot_ai_move2(context: &mut MonsterContext, distance: f64) {
    if distance != 0.0 {
        run_ai(context, &MonsterAi::Move, distance);
    }
    fixbot_turn(context);
}

/// Move-to-goal AI (`ai_movetogoal`).
fn fixbot_ai_movetogoal(context: &mut MonsterContext, distance: f64) {
    let actor = context.actor().clone();
    let origin = context.game.body_of(actor.clone()).origin;
    context.game.mission_monsters.fixbot_before_move.insert(actor, origin);
    context.move_to_goal(distance);
}

/// Facing AI (`ai_facing`).
fn fixbot_ai_facing(context: &mut MonsterContext, _distance: f64) {
    let goal = fixbot_goal(context);
    if in_front(context, goal.actor.id()) {
        context.set_move("fixbot_move_forward", false);
        return;
    }
    fixbot_turn(context);
}

/// Change to roam (`change_to_roam`).
fn fixbot_change_to_roam(context: &mut MonsterContext) {
    if fixbot_search(context) {
        return;
    }
    context.set_move("fixbot_move_roamgoal", false);
    if context.entity().spawnflags & 16 != 0 {
        fixbot_vertical_goal(context, true);
        context.entity_mut().spawnflags = 32;
    }
    if context.entity().spawnflags & 8 != 0 {
        fixbot_vertical_goal(context, false);
        context.entity_mut().spawnflags = 32;
    }
    if context.entity().spawnflags & 4 != 0 {
        context.set_move("fixbot_move_roamgoal", false);
        context.entity_mut().spawnflags = 32;
    }
    if context.entity().spawnflags == 0 {
        context.set_move("fixbot_move_stand2", false);
    }
}

/// Roam goal (`roam_goal`).
fn fixbot_roam_goal(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    let target = context.game.create("bot_goal", BTreeMap::new());
    context.game.require_entity_mut(&target).owner = Some(actor.clone());
    context.game.set_solid(target.clone(), Q2Solid::Box);
    let mut distance = 0;
    let mut chosen = vec3(0.0, 0.0, 0.0);
    for i in 0..12 {
        let yaw = if i < 6 {
            (i as f32) * 30.0
        } else {
            -((i - 6) as f32) * 30.0
        };
        let angles = vec3(body.angles.x, body.angles.y + yaw, body.angles.z);
        let trace = context.game.host.trace(&Q2TraceRequest {
            start: body.origin,
            end: add3(
                body.origin,
                scale3(angles_vectors(angles).forward, 8192.0),
            ),
            bounds: None,
            ignore: Some(actor.clone()),
            mask: MASK_SHOT,
            exclude: Vec::new(),
        });
        let size = length3(sub3(body.origin, trace.end)).trunc() as i32;
        if size > distance {
            distance = size;
            chosen = trace.end;
        }
    }
    let mut moved = context.game.body_of(target.clone());
    moved.origin = chosen;
    context.game.write_body(target.clone(), &moved, false);
    context.entity_mut().goal = Some(target.clone());
    context.entity_mut().enemy = Some(target);
    context.set_move("fixbot_move_turn", false);
}

/// Fly vertical 2 (`fly_vertical2`).
fn fixbot_fly_vertical2(context: &mut MonsterContext) {
    if fixbot_turn(context) < 32.0 {
        fixbot_leave_goal(context);
    }
}

/// Fly vertical (`fly_vertical`).
fn fixbot_fly_vertical(context: &mut MonsterContext) {
    fixbot_turn(context);
    let frame = context.entity().frame;
    if frame == fixbot_frame::LANDING_58 || frame == fixbot_frame::TAKEOFF_16 {
        fixbot_leave_goal(context);
    }
    let actor = context.actor().clone();
    let body = context.game.body_of(actor);
    let tilted = vec3(body.angles.x + 90.0, body.angles.y, body.angles.z);
    let direction = angles_vectors(tilted).forward;
    let spread = (500 + context.entity().frame - fixbot_frame::TAKEOFF_01) as f64;
    let fire_shotgun = context.weapons.fire_shotgun;
    for _ in 0..10 {
        let actor = context.actor().clone();
        fire_shotgun(
            actor,
            &mut *context.game,
            body.origin,
            direction,
            2.0,
            1.0,
            spread,
            spread,
            1,
            37,
        );
    }
}

/// Use scanner (`use_scanner`).
fn fixbot_use_scanner(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let origin = context.game.body_of(actor.clone()).origin;
    let nearby = context.game.host.nearby(origin, 1024.0);
    for candidate_id in nearby {
        let candidate = context.game.entity(&candidate_id).cloned();
        let Some(candidate) = candidate else {
            continue;
        };
        if health(&mut *context.game, Some(&candidate_id)) < 100.0
            || candidate.classname != "object_repair"
            || !visible(context, Some(&candidate_id))
        {
            continue;
        }
        let old = fixbot_goal(context);
        if old.classname == "bot_goal" {
            context.game.schedule(old.actor.id().clone(), 0.1, free_q2_entity);
        }
        context.entity_mut().goal = Some(candidate_id.clone());
        context.entity_mut().enemy = Some(candidate_id.clone());
        let origin = context.game.body_of(actor.clone()).origin;
        let candidate_body = context.game.body_of(candidate_id);
        if length3(sub3(origin, candidate_body.origin)) < 32.0 {
            context.set_move("fixbot_move_weld_start", false);
        }
        return;
    }
    let target = fixbot_goal(context);
    let origin = context.game.body_of(actor.clone()).origin;
    let target_body = context.game.body_of(target.actor.id().clone());
    if length3(sub3(origin, target_body.origin)) < 32.0 {
        if target.classname == "object_repair" {
            context.set_move("fixbot_move_weld_start", false);
        } else {
            fixbot_leave_goal(context);
        }
        return;
    }
    let Some(previous) = context.game.mission_monsters.fixbot_before_move.get(&actor).copied()
    else {
        panic!("Fixbot scanner must follow source movement in the same frame");
    };
    let origin = context.game.body_of(actor).origin;
    if length3(sub3(origin, previous)).trunc() as i32 == 0 {
        if target.classname == "object_repair" {
            context.set_move("fixbot_move_stand", false);
        } else {
            fixbot_leave_goal(context);
        }
    }
}

/// Weld state (`weldstate`).
fn fixbot_weldstate(context: &mut MonsterContext) {
    if context.entity().frame == fixbot_frame::WELDSTART_10 {
        context.set_move("fixbot_move_weld", false);
        return;
    }
    if context.entity().frame == fixbot_frame::WELDMIDDLE_07 {
        let target = fixbot_goal(context);
        let target_id = target.actor.id().clone();
        let hp = health(&mut *context.game, Some(&target_id));
        if hp < 0.0 {
            let enemy = context.entity().enemy.clone();
            if let Some(enemy) = enemy {
                context.game.require_entity_mut(&enemy).owner = None;
            }
            context.set_move("fixbot_move_weld_end", false);
            return;
        }
        let owned = context.game.require_entity(&target_id).actor.clone();
        context.game.host.combat().set_health(&owned, hp - 10.0);
        return;
    }
    context.entity_mut().goal = None;
    context.entity_mut().enemy = None;
    context.set_move("fixbot_move_stand", false);
}

/// Fire welder (`fixbot_fire_welder`).
fn fixbot_fire_welder(context: &mut MonsterContext) {
    if context.entity().enemy.is_none() {
        return;
    }
    let start = project_flash(context, vec3(24.0, -0.8, -10.0), None);
    let color = 0xe0 + (context.game.random() * 8.0).floor() as i32;
    context.game.host_emit(Q2PresentationEvent::Effect(Q2EffectEvent {
        effect: "q2:welding-sparks".to_string(),
        origin: start,
        direction: vec3(0.0, 0.0, 0.0),
        count: 10,
        color,
    }));
    if context.game.random() > 0.8 {
        let random = context.game.random();
        let actor = context.actor().clone();
        context.game.sound(
            &actor,
            if random < 0.33 {
                "misc/welder1.wav"
            } else if random < 0.66 {
                "misc/welder2.wav"
            } else {
                "misc/welder3.wav"
            },
            2,
            1.0,
            2.0,
        );
    }
}

/// Fire blaster (`fixbot_fire_blaster`).
fn fixbot_fire_blaster(context: &mut MonsterContext) {
    if !visible(context, None) {
        context.set_move("fixbot_move_run", false);
    }
    let Some((start, direction)) = monster_shot(context, 58, 0.0) else {
        return;
    };
    let fire_blaster = context.weapons.fire_blaster;
    let actor = context.actor().clone();
    fire_blaster(
        actor,
        &mut *context.game,
        start,
        direction,
        15.0,
        1000.0,
        8,
        false,
        Mod::BLASTER,
    );
    monster_muzzle(context, 58, direction, start);
}

/// Fire laser (`fixbot_fire_laser`).
fn fixbot_fire_laser(context: &mut MonsterContext) {
    let enemy = context.entity().enemy.clone();
    let Some(enemy_id) = enemy else {
        return;
    };
    if context.game.entity(&enemy_id).is_none() {
        return;
    }
    let gib_health = context
        .game
        .monsters
        .states
        .get(&enemy_id)
        .map(|patient| patient.gib_health);
    let Some(gib_health) = gib_health else {
        return;
    };
    if health(&mut *context.game, Some(&enemy_id)) <= gib_health {
        context.state_mut().medic = false;
        context.set_move("fixbot_move_stand", false);
        return;
    }
    let actor = context.actor().clone();
    context.game.sound(&actor, "misc/lasfly.wav", 0, 1.0, 3.0);
    let body = context.game.body_of(actor.clone());
    let target_body = context.game.body_of(enemy_id.clone());
    let angles = vector_angles(sub3(target_body.origin, body.origin));
    let origin = add3(
        body.origin,
        scale3(angles_vectors(angles).forward, 16.0),
    );
    monster_dabeam(
        &actor,
        &mut *context.game,
        Some(enemy_id.clone()),
        origin,
        angles,
        -1.0,
        true,
    );
    let mass = context
        .game
        .host
        .combat()
        .read(&enemy_id)
        .map(|state| state.mass)
        .unwrap_or(0.0);
    if health(&mut *context.game, Some(&enemy_id)) <= mass / 10.0 {
        if let Some(patient) = context.game.monsters.states.get_mut(&enemy_id) {
            patient.resurrecting = true;
        }
        return;
    }
    let target_body = context.game.body_of(enemy_id.clone());
    let trace = context.game.host.trace(&Q2TraceRequest {
        start: target_body.origin,
        end: scale3(angles_vectors(target_body.angles).up, 48.0),
        bounds: Some(target_body.bounds),
        ignore: Some(actor.clone()),
        mask: monster_solid_mask(&*context.game),
        exclude: Vec::new(),
    });
    if let TraceHit::Actor { actor: hit } = &trace.hit {
        let damageable = context
            .game
            .host
            .combat()
            .read(hit)
            .is_some_and(|state| state.can_take_damage);
        if damageable {
            if let Some(victim) = context.game.host.actors().resolve_owned(hit) {
                context.game.host.combat().set_health(&victim, -1000.0);
            }
            return;
        }
    }
    {
        let target = context.game.require_entity_mut(&enemy_id);
        target.spawnflags = 0;
        target.target = String::new();
        target.targetname = String::new();
        target.combat_target = String::new();
        target.death_target = String::new();
        target.owner = Some(actor.clone());
    }
    respawn_monster(&mut *context.game, enemy_id.clone());
    context.game.require_entity_mut(&enemy_id).owner = None;
    if let Some(restored) = context.game.monsters.states.get_mut(&enemy_id) {
        restored.resurrecting = false;
    }
    let mut moved = context.game.body_of(actor.clone());
    moved.origin.z += 1.0;
    context.game.write_body(actor, &moved, false);
    context.state_mut().medic = false;
    context.set_move("fixbot_move_stand", false);
}

/// Create the fixbot definition (`createFixbotDefinition`).
pub fn create_fixbot_definition() -> Q2MonsterDefinition {
    let mut definition = Q2MonsterDefinition::new(
        "monster_fixbot",
        "fixbot",
        "models/monsters/fixbot/tris.md2",
        150.0,
        0.0,
        150.0,
        Bounds {
            min: vec3(-32.0, -32.0, -24.0),
            max: vec3(32.0, 32.0, 24.0),
        },
        1.0,
        "fixbot_move_stand",
        fixbot_moves(),
        move_handler("fixbot_move_stand"),
        MonsterHandler::Callback(fixbot_walk),
        MonsterHandler::Callback(fixbot_run),
        MonsterHandler::Callback(fixbot_attack),
        fixbot_die,
    );
    definition.locomotion = Some(MonsterLocomotion::Fly);
    let mut source_callbacks = Q2CallbackDefinitions::default();
    source_callbacks.think.insert("G_FreeEdict", free_q2_entity);
    definition.source_callbacks = Some(source_callbacks);
    definition.pain = Some(fixbot_pain);
    definition.ai.insert("ai_move2".to_string(), fixbot_ai_move2);
    definition.ai.insert("ai_movetogoal".to_string(), fixbot_ai_movetogoal);
    definition.ai.insert("ai_facing".to_string(), fixbot_ai_facing);
    definition.callbacks.insert(
        "fixbot_run".to_string(),
        MonsterHandler::Callback(fixbot_run),
    );
    definition.callbacks.insert(
        "fixbot_dead".to_string(),
        MonsterHandler::Callback(finish_corpse_default),
    );
    definition.callbacks.insert(
        "fixbot_attack".to_string(),
        MonsterHandler::Callback(fixbot_attack),
    );
    definition.callbacks.insert(
        "change_to_roam".to_string(),
        MonsterHandler::Callback(fixbot_change_to_roam),
    );
    definition.callbacks.insert(
        "roam_goal".to_string(),
        MonsterHandler::Callback(fixbot_roam_goal),
    );
    definition.callbacks.insert(
        "fly_vertical2".to_string(),
        MonsterHandler::Callback(fixbot_fly_vertical2),
    );
    definition.callbacks.insert(
        "fly_vertical".to_string(),
        MonsterHandler::Callback(fixbot_fly_vertical),
    );
    definition.callbacks.insert(
        "use_scanner".to_string(),
        MonsterHandler::Callback(fixbot_use_scanner),
    );
    definition.callbacks.insert(
        "weldstate".to_string(),
        MonsterHandler::Callback(fixbot_weldstate),
    );
    definition.callbacks.insert(
        "fixbot_fire_welder".to_string(),
        MonsterHandler::Callback(fixbot_fire_welder),
    );
    definition.callbacks.insert(
        "fixbot_fire_blaster".to_string(),
        MonsterHandler::Callback(fixbot_fire_blaster),
    );
    definition.callbacks.insert(
        "fixbot_fire_laser".to_string(),
        MonsterHandler::Callback(fixbot_fire_laser),
    );
    definition
}
