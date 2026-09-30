//! Rogue medic (`src/content/q2/missionpacks/monsters/medic.ts`).
//!
//! Quake II rogue/m_medic.c. ZeniMax Media, GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3, add3, length3, scale3, sub3, vec3};

use super::combat::{cleanup_rogue_heal_target, rogue_heal_effects};
use super::rogue_common::{
    monster_mass, rogue_blocked_check_shot, rogue_duck_down, rogue_duck_hold,
    rogue_duck_up, rogue_monster_dodge, source_trace_world,
};
use super::spawn::{
    check_rogue_ground_spawn_point, check_rogue_spawn_point,
    create_rogue_ground_monster, find_rogue_spawn_point, rogue_spawn_callbacks,
    rogue_spawn_grow,
};
use super::state::rogue_state;
use super::tables::rogue_medic::{medic_frame, medic_moves};
use super::types::mission_weapons;
use crate::q2::base::monsters::common::{
    HUMANOID_BOUNDS, begin_death, finish_corpse_default, monster_shot,
    move_handler,
};
use crate::q2::foundation::host::{Q2Edition, Q2Mode, Q2TraceRequest};
use crate::q2::foundation::monsters::ai::{
    angles_vectors, finish_dodge, health, monster_solid_mask, project_flash,
    target_distance, visible,
};
use crate::q2::foundation::monsters::perception::{default_check_attack, found_target};
use crate::q2::foundation::monsters::types::{
    MonsterAttackState, MonsterContext, MonsterHandler, MonsterSpawner,
    Q2MonsterDefinition, record_at,
};
use crate::q2::foundation::monsters::respawn_monster;
use crate::q2::foundation::weapons::types::Mod;
use crate::q2::rerelease::monsters::common::{blocked_check_platform, monster_flash};
use crate::q2::support::contracts::{
    DeathReaction, PainReaction, TraceHit, TraceResult,
};

/// Cable offsets (`cableOffsets`).
const CABLE_OFFSETS: [Vec3; 10] = [
    Vec3 { x: 45.0, y: -9.2, z: 15.5 },
    Vec3 { x: 48.4, y: -9.7, z: 15.2 },
    Vec3 { x: 47.8, y: -9.8, z: 15.8 },
    Vec3 { x: 47.3, y: -9.3, z: 14.3 },
    Vec3 { x: 45.4, y: -10.1, z: 13.1 },
    Vec3 { x: 41.9, y: -12.7, z: 12.0 },
    Vec3 { x: 37.8, y: -15.8, z: 11.2 },
    Vec3 { x: 34.3, y: -18.4, z: 10.7 },
    Vec3 { x: 32.7, y: -19.7, z: 10.4 },
    Vec3 { x: 32.7, y: -19.7, z: 10.4 },
];

/// Reinforcement positions (`reinforcementPositions`).
const REINFORCEMENT_POSITIONS: [Vec3; 5] = [
    Vec3 { x: 80.0, y: 0.0, z: 0.0 },
    Vec3 { x: 40.0, y: 60.0, z: 0.0 },
    Vec3 { x: 40.0, y: -60.0, z: 0.0 },
    Vec3 { x: 0.0, y: 80.0, z: 0.0 },
    Vec3 { x: 0.0, y: -80.0, z: 0.0 },
];

/// Reinforcements (`reinforcements`).
const REINFORCEMENTS: [&str; 7] = [
    "monster_soldier_light",
    "monster_soldier",
    "monster_soldier_ss",
    "monster_infantry",
    "monster_gunner",
    "monster_medic",
    "monster_gladiator",
];

/// Reinforcement bounds (`reinforcementBounds`).
fn reinforcement_bounds(index: i32) -> Bounds {
    if index == 6 {
        Bounds {
            min: vec3(-32.0, -32.0, -24.0),
            max: vec3(32.0, 32.0, 64.0),
        }
    } else {
        HUMANOID_BOUNDS
    }
}

/// Angle mod (`anglemod`).
fn angle_mod(angle: f64) -> f64 {
    ((angle * 65536.0 / 360.0).trunc() as i64 & 65535) as f64 * 360.0 / 65536.0
}

/// Pick a rogue coop target (`pickRogueCoopTarget`).
pub fn pick_rogue_coop_target(context: &mut MonsterContext) -> Option<ActorId> {
    if context.game.options.mode != Q2Mode::Coop {
        return None;
    }
    let players: Vec<ActorId> = context
        .game
        .host
        .players()
        .into_iter()
        .filter(|actor| {
            context.game.host.actors().is_live(actor)
                && context.game.host.is_player(actor)
                && visible(context, Some(actor))
        })
        .collect();
    if players.is_empty() {
        return None;
    }
    let pick = (context.game.random() * players.len() as f64).floor() as usize;
    Some(record_at(&players, pick.min(players.len() - 1)).clone())
}

/// Medic sound (`medicSound`).
fn medic_sound(
    context: &mut MonsterContext,
    normal: &str,
    commander: &str,
    channel: i32,
    attenuation: f64,
) {
    let actor = context.actor().clone();
    let path = if monster_mass(context) == 400.0 {
        format!("medic/{normal}.wav")
    } else {
        format!("medic_commander/{commander}.wav")
    };
    context.game.sound(&actor, &path, channel, 1.0, attenuation);
}

/// Cleanup (`cleanup`).
fn medic_cleanup(context: &mut MonsterContext, change_frame: bool) {
    let enemy = context.entity().enemy.clone();
    if let Some(target) = enemy {
        cleanup_rogue_heal_target(&mut *context.game, &target);
    }
    if change_frame {
        context.state_mut().next_frame = medic_frame::ATTACK52;
    }
}

/// Abort (`abort`).
fn medic_abort(context: &mut MonsterContext, change_frame: bool, gib: bool, mark: bool) {
    medic_cleanup(context, change_frame);
    let actor = context.actor().clone();
    let enemy = context.entity().enemy.clone();
    if let Some(target) = &enemy {
        if mark {
            let bad_medic1 = rogue_state(&mut *context.game, target).bad_medic1.clone();
            let previous = bad_medic1
                .as_ref()
                .and_then(|id| context.game.entity(id))
                .cloned();
            let bad = previous.is_some_and(|previous| previous.classname.starts_with("monster_medic"));
            if bad {
                rogue_state(&mut *context.game, target).bad_medic2 = Some(actor.clone());
            } else {
                rogue_state(&mut *context.game, target).bad_medic1 = Some(actor.clone());
            }
        }
    }
    if let Some(target) = &enemy {
        if gib {
            let threshold = context
                .game
                .monsters
                .states
                .get(target)
                .map(|state| state.gib_health)
                .unwrap_or(0.0);
            let origin = context.game.body_of(target.clone()).origin;
            context.game.damage(
                target.clone(),
                actor.clone(),
                Some(actor.clone()),
                if threshold == 0.0 { 500.0 } else { -threshold },
                0.0,
                vec3(0.0, 0.0, 0.0),
                origin,
                vec3(0.0, 0.0, 1.0),
                0,
                0,
                None,
            );
        }
    }
    context.state_mut().medic = false;
    let old_enemy = context.state().old_enemy.clone();
    let live = old_enemy.as_ref().is_some_and(|old| context.game.host.actors().is_live(old));
    context.entity_mut().enemy = if live { old_enemy } else { None };
    rogue_state(&mut *context.game, &actor).medic_tries = 0;
}

/// Find dead (`findDead`).
///
/// The donor compares callback identity the same way.
#[allow(unpredictable_function_pointer_comparisons)]
fn medic_find_dead(context: &mut MonsterContext) -> Option<ActorId> {
    let actor = context.actor().clone();
    let origin = context.game.body_of(actor.clone()).origin;
    let radius = if context.state().stand_ground { 400.0 } else { 1024.0 };
    let nearby = context.game.host.nearby(origin, radius);
    let flies_on = context.game.source_callbacks.resolve_think(Some("M_FliesOn"));
    let flies_off = context.game.source_callbacks.resolve_think(Some("M_FliesOff"));
    let mut best: Option<ActorId> = None;
    let mut best_health = 0.0;
    for candidate_id in nearby {
        let target = context.game.entity(&candidate_id).cloned();
        let Some(target) = target else {
            continue;
        };
        if target.actor.id() == &actor
            || target.server_flags & 4 == 0
            || target.classname.starts_with("player")
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
        let patient = rogue_state(&mut *context.game, &candidate_id);
        let (bad1, bad2, healer) =
            (patient.bad_medic1.clone(), patient.bad_medic2.clone(), patient.healer.clone());
        if bad1 == Some(actor.clone()) || bad2 == Some(actor.clone()) {
            continue;
        }
        if let Some(healer_id) = healer {
            let healer_entity = context.game.entity(&healer_id).cloned();
            let healing = healer_entity.is_some_and(|healer| {
                health(&mut *context.game, Some(&healer_id)) > 0.0
                    && healer.server_flags & 4 != 0
                    && context
                        .game
                        .monsters
                        .states
                        .get(&healer_id)
                        .is_some_and(|state| state.medic)
            });
            if healing {
                continue;
            }
        }
        if health(&mut *context.game, Some(&candidate_id)) > 0.0
            || target.next_think.is_some() && target.think != flies_on && target.think != flies_off
            || !visible(context, Some(&candidate_id))
        {
            continue;
        }
        let target_body = context.game.body_of(candidate_id.clone());
        if length3(sub3(origin, target_body.origin)) <= 32.0 {
            continue;
        }
        if best.is_none() || target.max_health > best_health {
            best_health = target.max_health;
            best = Some(candidate_id);
        }
    }
    if best.is_some() {
        let timestamp = context.game.host.now() + 10.0;
        context.entity_mut().timestamp = timestamp;
    }
    best
}

/// Acquire (`acquire`).
fn medic_acquire(context: &mut MonsterContext) -> bool {
    let Some(target) = medic_find_dead(context) else {
        return false;
    };
    let actor = context.actor().clone();
    let enemy = context.entity().enemy.clone();
    context.state_mut().old_enemy = enemy;
    context.entity_mut().enemy = Some(target.clone());
    rogue_state(&mut *context.game, &target).healer = Some(actor);
    context.state_mut().medic = true;
    found_target(context);
    true
}

/// Run (`run`).
fn medic_run(context: &mut MonsterContext) {
    finish_dodge(context);
    if !context.state().medic && medic_acquire(context) {
        return;
    }
    let stand_ground = context.state().stand_ground;
    context.set_move(
        if stand_ground {
            "medic_move_stand"
        } else {
            "medic_move_run"
        },
        false,
    );
}

/// Idle (`idle`).
fn medic_idle(context: &mut MonsterContext) {
    medic_sound(context, "idle", "medidle", 2, 2.0);
    if context.state().old_enemy.is_none() {
        medic_acquire(context);
    }
}

/// Attack (`attack`).
fn medic_attack(context: &mut MonsterContext) {
    finish_dodge(context);
    let actor = context.actor().clone();
    let melee_range = target_distance(context) < 80.0;
    if rogue_state(&mut *context.game, &actor).blocked {
        context.set_move("medic_move_callReinforcements", false);
        rogue_state(&mut *context.game, &actor).blocked = false;
    }
    let random = context.game.random();
    let commander = monster_mass(context) > 400.0;
    if context.state().medic {
        context.set_move(
            if commander && random > 0.8 && context.state().monster_slots > 2 {
                "medic_move_callReinforcements"
            } else {
                "medic_move_attackCable"
            },
            false,
        );
        return;
    }
    context.set_move(
        if context.state().attack_state == MonsterAttackState::Blind
            || commander && random > 0.2 && !melee_range && context.state().monster_slots > 2
        {
            "medic_move_callReinforcements"
        } else {
            "medic_move_attackBlaster"
        },
        false,
    );
}

/// Attacking (`attacking`).
fn medic_attacking(context: &mut MonsterContext) -> bool {
    let current = context.state().current_move.name.clone();
    current == "medic_move_attackHyperBlaster"
        || current == "medic_move_attackCable"
        || current == "medic_move_attackBlaster"
        || current == "medic_move_callReinforcements"
}

/// Duck (`duck`).
fn medic_duck_inner(context: &mut MonsterContext, eta: f64) {
    if context.state().medic {
        return;
    }
    if medic_attacking(context) {
        context.state_mut().ducked = false;
        return;
    }
    let wait = context.game.host.now() + eta
        + if context.game.options.skill == 0 {
            1.0
        } else {
            0.1 * f64::from(3 - context.game.options.skill)
        };
    context.state_mut().duck_wait = wait;
    rogue_duck_down(context);
    context.state_mut().next_frame = medic_frame::DUCK1;
    context.set_move("medic_move_duck", false);
}

/// Sidestep (`sidestep`).
fn medic_sidestep_inner(context: &mut MonsterContext) {
    if medic_attacking(context) && context.game.options.skill != 0 {
        context.state_mut().dodging = false;
        return;
    }
    if context.state().current_move.name != "medic_move_run" {
        context.set_move("medic_move_run", false);
    }
}

/// Cable (`cable`).
fn medic_cable(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let enemy = context.entity().enemy.clone();
    let target = enemy.as_ref().and_then(|enemy| context.game.entity(enemy)).cloned();
    let Some(target) = target else {
        medic_abort(context, true, false, false);
        return;
    };
    let target_id = target.actor.id().clone();
    if target.effects & 2 != 0
        || context.game.host.is_player(&target_id)
        || health(&mut *context.game, Some(&target_id)) > 0.0
    {
        medic_abort(context, true, false, false);
        return;
    }
    let offset = *record_at(&CABLE_OFFSETS, (context.entity().frame - medic_frame::ATTACK42) as usize);
    let start = project_flash(context, offset, None);
    let target_body = context.game.body_of(target_id.clone());
    if length3(sub3(start, target_body.origin)) < 32.0 {
        medic_abort(context, true, true, false);
        return;
    }
    let trace = context.game.host.trace(&Q2TraceRequest {
        start,
        end: target_body.origin,
        bounds: None,
        ignore: Some(actor.clone()),
        mask: 3,
        exclude: Vec::new(),
    });
    if trace.fraction != 1.0
        && !matches!(&trace.hit, TraceHit::Actor { actor } if *actor == target_id)
    {
        if source_trace_world(&mut *context.game, &trace) {
            if rogue_state(&mut *context.game, &actor).medic_tries > 1 {
                medic_abort(context, true, false, true);
            } else {
                rogue_state(&mut *context.game, &actor).medic_tries += 1;
                medic_cleanup(context, true);
            }
            return;
        }
        medic_abort(context, true, false, false);
        return;
    }
    let frame = context.entity().frame;
    if frame == medic_frame::ATTACK43 {
        let path = if monster_mass(context) == 400.0 {
            "medic/medatck3.wav"
        } else {
            "medic_commander/medatck3a.wav"
        };
        context.game.sound(&target_id, path, 0, 1.0, 1.0);
        if context.game.monsters.states.contains_key(&target_id) {
            if let Some(patient) = context.game.monsters.states.get_mut(&target_id) {
                patient.resurrecting = true;
                patient.can_take_damage = false;
            }
            let mut patient_context = MonsterContext::new(target_id.clone(), &mut *context.game);
            rogue_heal_effects(&mut patient_context);
        }
        let owned = context.game.require_entity(&target_id).actor.clone();
        context.game.host.combat().set_traits(
            &owned,
            &crate::q2::support::contracts::CombatTraitChanges {
                can_take_damage: Some(false),
                ..crate::q2::support::contracts::CombatTraitChanges::default()
            },
        );
    } else if frame == medic_frame::ATTACK50 {
        {
            let target = context.game.require_entity_mut(&target_id);
            target.spawnflags = 0;
            target.target = String::new();
            target.targetname = String::new();
            target.combat_target = String::new();
            target.death_target = String::new();
        }
        if let Some(flags) = context.game.monsters.states.get_mut(&target_id) {
            flags.ignore_shots = false;
            flags.do_not_count = false;
            flags.spawned_by = MonsterSpawner::None;
            flags.good_guy = false;
            flags.target_anger = false;
            flags.brutal = false;
            flags.medic = false;
            flags.resurrecting = false;
            flags.stand_ground = false;
            flags.temporary_stand_ground = false;
            flags.hold_frame = false;
            flags.ducked = false;
            flags.dodging = false;
            flags.charging = false;
            flags.manual_steering = false;
            flags.combat_point = false;
            flags.lost_sight = false;
            flags.pursue_next = false;
            flags.pursue_temporary = false;
            flags.pursuit_last_seen = false;
            flags.sound_target = None;
        }
        rogue_state(&mut *context.game, &target_id).healer = Some(actor.clone());
        let mask = monster_solid_mask(&*context.game);
        let target_body = context.game.body_of(target_id.clone());
        let mut bounds = target_body.bounds;
        bounds.max.z += 48.0;
        let clear = context.game.host.trace(&Q2TraceRequest {
            start: target_body.origin,
            end: target_body.origin,
            bounds: Some(bounds),
            ignore: Some(target_id.clone()),
            mask,
            exclude: Vec::new(),
        });
        if clear.start_solid || clear.all_solid || !source_trace_world(&mut *context.game, &clear) {
            medic_abort(context, true, true, false);
            return;
        }
        if let Some(previous) = context.game.monsters.states.get_mut(&target_id) {
            previous.do_not_count = true;
        }
        respawn_monster(&mut *context.game, target_id.clone());
        let think = context.game.require_entity(&target_id).think;
        if let Some(think) = think {
            let now = context.game.host.now();
            context.game.require_entity_mut(&target_id).next_think = Some(now);
            think(target_id.clone(), &mut *context.game);
        }
        if let Some(revived) = context.game.monsters.states.get_mut(&target_id) {
            revived.resurrecting = false;
            revived.ignore_shots = true;
            revived.do_not_count = true;
        }
        context.game.require_entity_mut(&target_id).effects &= !0x4000;
        rogue_state(&mut *context.game, &target_id).healer = None;
        let old_enemy = context.state().old_enemy.clone();
        let live = old_enemy.as_ref().is_some_and(|old| {
            context.game.host.actors().is_live(old) && health(&mut *context.game, Some(old)) > 0.0
        });
        if live {
            let old_enemy = old_enemy.expect("medic revive enemy");
            context.game.require_entity_mut(&target_id).enemy = Some(old_enemy);
            let mut revived_context = MonsterContext::new(target_id.clone(), &mut *context.game);
            found_target(&mut revived_context);
        } else {
            context.game.require_entity_mut(&target_id).enemy = None;
            let mut revived_context = MonsterContext::new(target_id.clone(), &mut *context.game);
            if !revived_context.find_target() {
                let pause = revived_context.game.host.now() + 100000000.0;
                revived_context.state_mut().pause_time = pause;
                revived_context.stand();
            }
            context.entity_mut().enemy = None;
            context.state_mut().old_enemy = None;
            if !context.find_target() {
                let pause = context.game.host.now() + 100000000.0;
                context.state_mut().pause_time = pause;
                context.stand();
            }
        }
    } else if frame == medic_frame::ATTACK44 {
        medic_sound(context, "medatck4", "medatck4a", 1, 1.0);
    }
    let current = context.entity().enemy.clone();
    let current_target = current.as_ref().and_then(|enemy| context.game.entity(enemy)).cloned();
    let Some(current_target) = current_target else {
        return;
    };
    let body = context.game.body_of(current_target.actor.id().clone());
    let self_body = context.game.body_of(actor.clone());
    context.game.host_emit(crate::q2::foundation::host::Q2PresentationEvent::MonsterBeam {
        effect: crate::q2::foundation::host::Q2MonsterBeam::Medic,
        actor: actor.clone(),
        start: add3(start, scale3(angles_vectors(self_body.angles).forward, 8.0)),
        end: vec3(body.origin.x, body.origin.y, body.origin.z + (body.bounds.min.z + body.bounds.max.z) / 2.0),
    });
}

/// Spawn slots (`eachSpawn`).
///
/// The donor visits slots through a callback; the port precomputes the
/// pure trace results and lets each caller iterate them.
fn medic_spawn_slots(context: &mut MonsterContext, behind: bool) -> Vec<(Vec3, Bounds, i32)> {
    let actor = context.actor().clone();
    let strength = rogue_state(&mut *context.game, &actor).summon_strength;
    let count = if strength == 0 { 1 } else { strength - 1 + strength % 2 };
    let mut slots = Vec::new();
    for i in 0..count {
        let index = strength - i - i % 2;
        let position = *record_at(&REINFORCEMENT_POSITIONS, i as usize);
        let bounds = reinforcement_bounds(index);
        let offset = if behind {
            vec3(-position.x, -position.y, position.z)
        } else {
            position
        };
        let point = project_flash(context, offset, None);
        let raised = vec3(point.x, point.y, point.z + 10.0);
        if let Some(spawn) = find_rogue_spawn_point(&mut *context.game, raised, bounds, 32.0) {
            slots.push((spawn, bounds, index));
        }
    }
    slots
}

/// Hook launch (`medic_hook_launch`).
fn medic_hook_launch(context: &mut MonsterContext) {
    medic_sound(context, "medatck2", "medatck2c", 1, 1.0);
}

/// Hook retract (`medic_hook_retract`).
fn medic_hook_retract(context: &mut MonsterContext) {
    medic_sound(context, "medatck5", "medatck5a", 1, 1.0);
    context.state_mut().medic = false;
    let old_enemy = context.state().old_enemy.clone();
    if old_enemy.as_ref().is_some_and(|old| context.game.host.actors().is_live(old)) {
        context.entity_mut().enemy = old_enemy;
    } else {
        context.entity_mut().enemy = None;
        context.state_mut().old_enemy = None;
        if !context.find_target() {
            let pause = context.game.host.now() + 100000000.0;
            context.state_mut().pause_time = pause;
            context.stand();
        }
    }
}

/// Continue (`medic_continue`).
fn medic_continue(context: &mut MonsterContext) {
    if visible(context, None) && context.game.random() <= 0.95 {
        context.set_move("medic_move_attackHyperBlaster", false);
    }
}

/// Fire blaster (`medic_fire_blaster`).
fn medic_fire_blaster(context: &mut MonsterContext) {
    let Some((start, direction)) = monster_shot(context, 60, 0.0) else {
        return;
    };
    let frame = context.entity().frame;
    let effects = if frame == medic_frame::ATTACK9 || frame == medic_frame::ATTACK12 {
        8
    } else if frame == medic_frame::ATTACK19
        || frame == medic_frame::ATTACK22
        || frame == medic_frame::ATTACK25
        || frame == medic_frame::ATTACK28
    {
        64
    } else {
        0
    };
    let tesla = if context.game.options.edition == Q2Edition::Rerelease {
        "tesla_mine"
    } else {
        "tesla"
    };
    let enemy = context.entity().enemy.clone();
    let is_tesla = enemy.as_ref().and_then(|enemy| context.game.entity(enemy)).is_some_and(|enemy| enemy.classname == tesla);
    let damage = if is_tesla { 3.0 } else { 2.0 };
    let commander = monster_mass(context) > 400.0;
    let actor = context.actor().clone();
    if commander {
        let weapons = mission_weapons(&*context.game);
        weapons.fire_blaster2(actor, &mut *context.game, start, direction, damage, 1000.0, effects);
    } else {
        let fire_blaster = context.weapons.fire_blaster;
        fire_blaster(actor, &mut *context.game, start, direction, damage, 1000.0, effects, false, Mod::BLASTER);
    }
    monster_flash(context, if commander { 146 } else { 60 }, start, direction);
}

/// Start spawn (`medic_start_spawn`).
fn medic_start_spawn(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "medic_commander/monsterspawn1.wav", 1, 1.0, 1.0);
    context.state_mut().next_frame = medic_frame::ATTACK48;
}

/// Determine spawn (`medic_determine_spawn`).
fn medic_determine_spawn(context: &mut MonsterContext) {
    let lucky = context.game.random();
    let delta = if lucky < 0.05 {
        -3
    } else if lucky < 0.15 {
        -2
    } else if lucky < 0.3 {
        -1
    } else if lucky > 0.95 {
        3
    } else if lucky > 0.85 {
        2
    } else if lucky > 0.7 {
        1
    } else {
        0
    };
    let strength = (i32::from(context.game.options.skill) + delta).max(0);
    let actor = context.actor().clone();
    rogue_state(&mut *context.game, &actor).summon_strength = strength;
    let mut success = false;
    for (point, bounds, _) in medic_spawn_slots(context, false) {
        success = check_rogue_ground_spawn_point(&mut *context.game, point, bounds, 256.0, -1.0);
        if success {
            break;
        }
    }
    if !success {
        for (point, bounds, _) in medic_spawn_slots(context, true) {
            success = check_rogue_ground_spawn_point(&mut *context.game, point, bounds, 256.0, -1.0);
            if success {
                break;
            }
        }
        if success {
            context.state_mut().manual_steering = true;
            let actor = context.actor().clone();
            let yaw = angle_mod(f64::from(context.game.body_of(actor).angles.y)) + 180.0;
            context.state_mut().ideal_yaw = if yaw > 360.0 { yaw - 360.0 } else { yaw };
        }
    }
    if !success {
        context.state_mut().next_frame = medic_frame::ATTACK53;
    }
}

/// Spawn grows (`medic_spawngrows`).
fn medic_spawngrows(context: &mut MonsterContext) {
    if context.state().manual_steering {
        let actor = context.actor().clone();
        let yaw = angle_mod(f64::from(context.game.body_of(actor).angles.y));
        if (yaw - context.state().ideal_yaw).abs() > 0.1 {
            context.state_mut().hold_frame = true;
            return;
        }
        context.state_mut().hold_frame = false;
        context.state_mut().manual_steering = false;
    }
    let mut success = false;
    for (point, bounds, index) in medic_spawn_slots(context, false) {
        if check_rogue_ground_spawn_point(&mut *context.game, point, bounds, 256.0, -1.0) {
            success = true;
            rogue_spawn_grow(&mut *context.game, point, if index > 3 { 1 } else { 0 });
        }
    }
    if !success {
        context.state_mut().next_frame = medic_frame::ATTACK53;
    }
}

/// Finish spawn (`medic_finish_spawn`).
fn medic_finish_spawn(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let strength = rogue_state(&mut *context.game, &actor).summon_strength.abs();
    rogue_state(&mut *context.game, &actor).summon_strength = strength;
    for (point, bounds, index) in medic_spawn_slots(context, false) {
        if !check_rogue_spawn_point(&mut *context.game, point, bounds) {
            continue;
        }
        let angles = context.game.body_of(actor.clone()).angles;
        let child = create_rogue_ground_monster(
            &mut *context.game,
            point,
            angles,
            bounds,
            record_at(&REINFORCEMENTS, index as usize),
            256.0,
        );
        let Some(child) = child else {
            continue;
        };
        let think = context.game.require_entity(&child).think;
        if let Some(think) = think {
            let now = context.game.host.now();
            context.game.require_entity_mut(&child).next_think = Some(now);
            think(child.clone(), &mut *context.game);
        }
        if !context.game.monsters.states.contains_key(&child) {
            panic!("Medic reinforcement has no shared controller");
        }
        if let Some(state) = context.game.monsters.states.get_mut(&child) {
            state.ignore_shots = true;
            state.do_not_count = true;
            state.spawned_by = MonsterSpawner::Medic;
            state.commander = Some(actor.clone());
        }
        context.state_mut().monster_slots -= 1;
        let mut enemy = if context.state().medic {
            context.state().old_enemy.clone()
        } else {
            context.entity().enemy.clone()
        };
        if context.game.options.mode == Q2Mode::Coop {
            let mut child_context = MonsterContext::new(child.clone(), &mut *context.game);
            enemy = pick_rogue_coop_target(&mut child_context);
            let self_enemy = child_context.entity().enemy.clone();
            if enemy == self_enemy && enemy.is_some() {
                enemy = pick_rogue_coop_target(&mut child_context);
            }
            if enemy.is_none() {
                enemy = self_enemy;
            }
        }
        let live = enemy.as_ref().is_some_and(|enemy| {
            context.game.host.actors().is_live(enemy) && health(&mut *context.game, Some(enemy)) > 0.0
        });
        if live {
            let enemy = enemy.expect("medic reinforcement enemy");
            context.game.require_entity_mut(&child).enemy = Some(enemy);
            let mut child_context = MonsterContext::new(child.clone(), &mut *context.game);
            found_target(&mut child_context);
        } else {
            context.game.require_entity_mut(&child).enemy = None;
            let mut child_context = MonsterContext::new(child.clone(), &mut *context.game);
            child_context.stand();
        }
    }
}

/// Sight (`sight`).
fn medic_sight(context: &mut MonsterContext) {
    medic_sound(context, "medsght1", "medsght", 2, 1.0);
}

/// Search (`search`).
fn medic_search(context: &mut MonsterContext) {
    medic_sound(context, "medsrch1", "medsrch", 2, 2.0);
    if context.state().old_enemy.is_none() {
        medic_acquire(context);
    }
}

/// Initialize (`initialize`).
fn medic_initialize(context: &mut MonsterContext) {
    context.state_mut().ignore_shots = true;
    if monster_mass(context) > 400.0 {
        context.entity_mut().skin = 2;
        let skill = context.game.options.skill;
        context.state_mut().monster_slots = if skill == 0 {
            3
        } else if skill == 1 {
            4
        } else {
            6
        };
    }
}

/// Pain (`pain`).
fn medic_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    let actor = context.actor().clone();
    let commander = monster_mass(context) > 400.0;
    finish_dodge(context);
    let max_health = context.entity().max_health;
    if health(&mut *context.game, Some(&actor)) < max_health / 2.0 {
        context.entity_mut().skin = if commander { 3 } else { 1 };
    }
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    if context.game.options.skill == 3 || context.state().medic {
        return;
    }
    if commander {
        if reaction.damage < 35.0 {
            medic_sound(context, "medpain1", "medpain1", 2, 1.0);
            return;
        }
        context.state_mut().manual_steering = false;
        context.state_mut().hold_frame = false;
        medic_sound(context, "medpain2", "medpain2", 2, 1.0);
        let heavy = context.game.random() < (reaction.damage * 0.005).min(0.5);
        context.set_move(if heavy { "medic_move_pain2" } else { "medic_move_pain1" }, false);
    } else {
        let first = context.game.random() < 0.5;
        context.set_move(if first { "medic_move_pain1" } else { "medic_move_pain2" }, false);
        medic_sound(context, if first { "medpain1" } else { "medpain2" }, "medpain2", 2, 1.0);
    }
    if context.state().ducked {
        rogue_duck_up(context);
    }
}

/// Die (`die`).
fn medic_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    let path = if monster_mass(context) == 400.0 {
        "medic/meddeth1.wav"
    } else {
        "medic_commander/meddeth.wav"
    };
    begin_death(context, reaction, path, "medic_move_death", 2, 4);
}

/// Dodge (`dodge`).
fn medic_dodge(
    context: &mut MonsterContext,
    attacker: &ActorId,
    eta: f64,
    trace: Option<&TraceResult>,
    _direct: bool,
) {
    rogue_monster_dodge(
        context,
        attacker,
        eta,
        trace,
        Some(medic_duck_inner),
        Some(medic_sidestep_inner),
    );
}

/// Duck slot (`duck`).
fn medic_duck(context: &mut MonsterContext, eta: f64) -> bool {
    medic_duck_inner(context, eta);
    true
}

/// Sidestep slot (`sidestep`).
fn medic_sidestep(context: &mut MonsterContext) -> bool {
    medic_sidestep_inner(context);
    true
}

/// Blocked (`blocked`).
fn medic_blocked(context: &mut MonsterContext, distance: f64) -> bool {
    let chance = 0.25 + 0.05 * f64::from(context.game.options.skill);
    rogue_blocked_check_shot(context, chance) || blocked_check_platform(context, distance)
}

/// Check attack (`checkAttack`).
fn medic_check_attack(context: &mut MonsterContext) -> bool {
    let actor = context.actor().clone();
    if context.state().medic {
        let enemy = context.entity().enemy.clone();
        if enemy.is_none()
            || enemy.as_ref().is_some_and(|enemy| !context.game.host.actors().is_live(enemy))
        {
            medic_abort(context, true, false, false);
            return false;
        }
        if context.entity().timestamp < context.game.host.now() {
            medic_abort(context, true, false, true);
            context.entity_mut().timestamp = 0.0;
            return false;
        }
        if target_distance(context) < 410.0 {
            medic_attack(context);
            return true;
        }
        context.state_mut().attack_state = MonsterAttackState::Straight;
        return false;
    }
    let enemy = context.entity().enemy.clone();
    if enemy.as_ref().is_some_and(|enemy| context.game.host.is_player(enemy))
        && !visible(context, None)
        && context.state().monster_slots > 2
    {
        context.state_mut().attack_state = MonsterAttackState::Blind;
        return true;
    }
    if context.game.random() < 0.8
        && context.state().monster_slots > 5
        && target_distance(context) > 150.0
    {
        rogue_state(&mut *context.game, &actor).blocked = true;
        context.state_mut().attack_state = MonsterAttackState::Missile;
        return true;
    }
    if context.game.options.skill > 0 && context.state().stand_ground {
        context.state_mut().attack_state = MonsterAttackState::Missile;
        return true;
    }
    default_check_attack(context)
}

/// Create rogue medic definitions (`createRogueMedicDefinitions`).
pub fn create_rogue_medic_definitions() -> Vec<Q2MonsterDefinition> {
    let mut definition = Q2MonsterDefinition::new(
        "monster_medic",
        "medic",
        "models/monsters/medic/tris.md2",
        300.0,
        -130.0,
        400.0,
        Bounds {
            min: vec3(-24.0, -24.0, -24.0),
            max: vec3(24.0, 24.0, 32.0),
        },
        1.0,
        "medic_move_stand",
        medic_moves(),
        move_handler("medic_move_stand"),
        move_handler("medic_move_walk"),
        MonsterHandler::Callback(medic_run),
        MonsterHandler::Callback(medic_attack),
        medic_die,
    );
    definition.idle = Some(MonsterHandler::Callback(medic_idle));
    definition.source_callbacks = Some(rogue_spawn_callbacks());
    definition.sight = Some(MonsterHandler::Callback(medic_sight));
    definition.search = Some(MonsterHandler::Callback(medic_search));
    definition.initialize = Some(MonsterHandler::Callback(medic_initialize));
    definition.pain = Some(medic_pain);
    definition.dodge = Some(medic_dodge);
    definition.duck = Some(medic_duck);
    definition.sidestep = Some(medic_sidestep);
    definition.blocked = Some(medic_blocked);
    definition.check_attack = Some(medic_check_attack);
    for (name, handler) in [
        ("medic_idle", MonsterHandler::Callback(medic_idle)),
        ("medic_run", MonsterHandler::Callback(medic_run)),
        ("medic_dead", MonsterHandler::Callback(finish_corpse_default)),
        ("monster_done_dodge", MonsterHandler::Callback(finish_dodge)),
        ("monster_duck_down", MonsterHandler::Callback(rogue_duck_down)),
        ("monster_duck_hold", MonsterHandler::Callback(rogue_duck_hold)),
        ("monster_duck_up", MonsterHandler::Callback(rogue_duck_up)),
        ("medic_hook_launch", MonsterHandler::Callback(medic_hook_launch)),
        ("medic_hook_retract", MonsterHandler::Callback(medic_hook_retract)),
        ("medic_cable_attack", MonsterHandler::Callback(medic_cable)),
        ("medic_continue", MonsterHandler::Callback(medic_continue)),
        ("medic_fire_blaster", MonsterHandler::Callback(medic_fire_blaster)),
        ("medic_start_spawn", MonsterHandler::Callback(medic_start_spawn)),
        ("medic_determine_spawn", MonsterHandler::Callback(medic_determine_spawn)),
        ("medic_spawngrows", MonsterHandler::Callback(medic_spawngrows)),
        ("medic_finish_spawn", MonsterHandler::Callback(medic_finish_spawn)),
    ] {
        definition.callbacks.insert(name.to_string(), handler);
    }
    let mut commander = definition.clone();
    commander.classname = "monster_medic_commander".to_string();
    commander.kind = "medic_commander".to_string();
    commander.health = 600.0;
    commander.mass = 600.0;
    commander.yaw_speed = Some(40.0);
    vec![definition, commander]
}
