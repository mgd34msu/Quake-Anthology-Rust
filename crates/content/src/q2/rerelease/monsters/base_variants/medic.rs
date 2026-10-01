//! Rerelease medic (`src/content/q2/rerelease/monsters/base-variants/medic.ts`).
//!
//! ZeniMax Media, GPL-2.0-or-later.

use std::collections::BTreeMap;

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3, add3, length3, scale3, sub3, vec3};

use super::super::common::{
    blocked_check_platform, chainfist, check_gib, monster_flash, reacts_to_pain,
    rerelease_random,
};
use super::super::spawn_placement::{
    check_rerelease_ground_spawn_point, find_rerelease_spawn_point,
};
use super::super::tables::medic::{medic_frame, medic_moves};
use crate::contract::{ArmorState, InventoryEntry, PoweredProtectionState, RegularArmorState};
use crate::q2::base::monsters::common::{monster_shot, move_handler};
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::fields::number_field;
use crate::q2::foundation::host::{
    Q2BeamEvent, Q2GameServices, Q2MonsterBeam, Q2MotionKind, Q2PresentationEvent,
    Q2Solid, Q2TraceRequest,
};
use crate::q2::foundation::monsters::ai::{
    angles_vectors, corpse, finish_dodge, health, monster_solid_mask, project_flash,
    set_duck, target_distance, visible,
};
use crate::q2::foundation::monsters::gibs::{Q2GibOptions, throw_gib};
use crate::q2::foundation::monsters::perception::{
    default_check_attack, found_target, hunt_target,
};
use crate::q2::foundation::monsters::types::{
    MonsterAttackState, MonsterContext, MonsterHandler, MonsterPowerArmor,
    MonsterSpawner, Q2MonsterDefinition, record_at,
};
use crate::q2::foundation::monsters::{monster_definition, respawn_monster};
use crate::q2::missionpacks::monsters::combat::{cleanup_rogue_heal_target, rogue_heal_effects};
use crate::q2::missionpacks::monsters::medic::pick_rogue_coop_target;
use crate::q2::missionpacks::monsters::power_armor::{
    PowerArmorKind, monster_power_armor,
};
use crate::q2::missionpacks::monsters::rogue_common::{monster_mass, source_trace_world};
use crate::q2::missionpacks::monsters::spawn::{check_rogue_spawn_point, create_rogue_monster};
use crate::q2::missionpacks::monsters::state::rogue_state;
use crate::q2::missionpacks::monsters::types::mission_weapons;
use crate::q2::support::contracts::{
    CombatTraitChanges, DeathReaction, PainReaction, TraceHit,
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

/// Default reinforcements (`defaultReinforcements`).
const DEFAULT_REINFORCEMENTS: &str = "monster_soldier_light 1;monster_soldier 2;monster_soldier_ss 2;monster_infantry 3;monster_gunner 4;monster_medic 5;monster_gladiator 6";

/// Parsed reinforcement (`rereleaseMedicReinforcements` entry).
pub struct MedicReinforcement {
    /// Classname.
    pub classname: String,
    /// Strength.
    pub strength: i32,
}

/// Reinforcement (`Reinforcement`).
struct Reinforcement {
    classname: String,
    strength: i32,
    bounds: Bounds,
}

/// Parse medic reinforcements (`rereleaseMedicReinforcements`).
pub fn rerelease_medic_reinforcements(fields: &BTreeMap<String, String>) -> Vec<MedicReinforcement> {
    let value = fields.get("reinforcements").map(String::as_str).unwrap_or(DEFAULT_REINFORCEMENTS);
    if value.is_empty() {
        return Vec::new();
    }
    value
        .split(';')
        .map(|entry| {
            let mut parts = entry.trim().split_whitespace();
            let classname = parts.next().unwrap_or("").to_string();
            let strength = parts.next().unwrap_or("0").parse::<i32>().unwrap_or(0);
            MedicReinforcement { classname, strength }
        })
        .collect()
}

/// Angle mod (`anglemod`).
fn medic_angle_mod(angle: f64) -> f64 {
    ((angle * 65536.0 / 360.0).trunc() as i32 & 65535) as f64 * 360.0 / 65536.0
}

/// Spawn grow laser think (`spawnGrowLaserThink`).
fn spawn_grow_laser_think(beam: ActorId, game: &mut Q2GameServices) {
    let owner = game.require_entity(&beam).owner.clone();
    let owner = owner.as_ref().and_then(|owner| game.entity(owner).cloned());
    let Some(owner) = owner else {
        game.remove_actor(beam);
        return;
    };
    let (scale, skin) = (owner.scale, owner.skin);
    let theta = game.host.random() * 2.0 * std::f64::consts::PI;
    let phi = (game.host.random() * 2.0 - 1.0).acos();
    let scatter = vec3(
        (phi.sin() * theta.cos()) as f32,
        (phi.sin() * theta.sin()) as f32,
        phi.cos() as f32,
    );
    let origin = game.body_of(beam.clone()).origin;
    game.require_entity_mut(&beam).pos2 = add3(origin, scale3(scatter, (scale * 9.0) as f32));
    game.link_actor(beam.clone());
    let end = game.require_entity(&beam).pos2;
    game.host_emit(Q2PresentationEvent::Beam(Q2BeamEvent {
        actor: beam.clone(),
        start: origin,
        end,
        width: 1.0,
        color: skin,
        visible: true,
    }));
    game.schedule(beam, 0.001, spawn_grow_laser_think);
}

/// Spawn grow think (`spawnGrowThink`).
fn spawn_grow_think(entity: ActorId, game: &mut Q2GameServices) {
    if game.host.now() >= game.require_entity(&entity).timestamp {
        let beam = game.require_entity(&entity).beam.clone();
        if let Some(beam) = beam {
            if game.entity(&beam).is_some() {
                let origin = game.body_of(beam.clone()).origin;
                let target = game.require_entity(&beam);
                let (end, skin) = (target.pos2, target.skin);
                game.host_emit(Q2PresentationEvent::Beam(Q2BeamEvent {
                    actor: beam.clone(),
                    start: origin,
                    end,
                    width: 1.0,
                    color: skin,
                    visible: false,
                }));
                game.remove_actor(beam);
            }
        }
        game.remove_actor(entity);
        return;
    }
    let mut moved = game.body_of(entity.clone());
    let angular = game.require_entity(&entity).angular_velocity;
    let frame_seconds = game.host.frame_seconds();
    moved.angles = add3(moved.angles, scale3(angular, frame_seconds as f32));
    game.write_body(entity.clone(), &moved, true);
    let target = game.require_entity(&entity);
    let (timestamp, wait, accel, decel) = (target.timestamp, target.wait, target.accel, target.decel);
    let t = 1.0 - (game.host.now() - (timestamp - wait)) / wait;
    let target = game.require_entity_mut(&entity);
    target.scale = 0.001f64.max(16.0f64.min((decel + t * (accel - decel)) / 16.0));
    target.alpha = t * t;
    game.show(entity.clone());
    game.schedule(entity, 0.1, spawn_grow_think);
}

/// Spawn grow (`spawnGrow`).
fn medic_spawn_grow(context: &mut MonsterContext, origin: Vec3, radius: f64) {
    let entity = context.game.create("spawngro", BTreeMap::new());
    let angles = vec3(
        rerelease_random(context).integer_max(360) as f32,
        rerelease_random(context).integer_max(360) as f32,
        rerelease_random(context).integer_max(360) as f32,
    );
    let mut moved = context.game.body_of(entity.clone());
    moved.origin = origin;
    moved.angles = angles;
    context.game.write_body(entity.clone(), &moved, false);
    let spin = vec3(
        rerelease_random(context).float_range(280.0, 360.0) * 2.0,
        rerelease_random(context).float_range(280.0, 360.0) * 2.0,
        rerelease_random(context).float_range(280.0, 360.0) * 2.0,
    );
    let now = context.game.host.now();
    let target = context.game.require_entity_mut(&entity);
    target.angular_velocity = spin;
    target.model = "models/items/spawngro3/tris.md2".to_string();
    target.render_flags = 32768;
    target.skin = 1;
    target.accel = radius;
    target.decel = radius * 2.0;
    target.scale = 0.001f64.max(8.0f64.min(radius / 16.0));
    target.wait = 1.0;
    target.timestamp = now + 1.0;
    context.game.set_solid(entity.clone(), Q2Solid::None);
    context.game.set_motion_kind(entity.clone(), Q2MotionKind::Stationary);
    context.game.schedule(entity.clone(), 0.1, spawn_grow_think);
    context.game.link_actor(entity.clone());
    context.game.show(entity.clone());
    let beam = context.game.create("spawngro_beam", BTreeMap::new());
    context.game.require_entity_mut(&entity).beam = Some(beam.clone());
    let target = context.game.require_entity_mut(&beam);
    target.owner = Some(entity);
    target.frame = 1;
    target.skin = 0x3030_3030;
    target.render_flags = 128 | 512 | 64;
    let mut moved = context.game.body_of(beam.clone());
    moved.origin = origin;
    context.game.write_body(beam.clone(), &moved, false);
    spawn_grow_laser_think(beam, &mut *context.game);
}

/// Medic sound (`medicSound`).
fn medic_sound(context: &mut MonsterContext, normal: &str, commander: &str, channel: i32, attenuation: f64) {
    let actor = context.actor().clone();
    let light = monster_mass(context) == 400.0;
    let path = if light {
        format!("medic/{normal}.wav")
    } else {
        format!("medic_commander/{commander}.wav")
    };
    context.game.sound(&actor, &path, channel, 1.0, attenuation);
}

/// Restore enemy (`restoreEnemy`).
fn medic_restore_enemy(context: &mut MonsterContext) -> bool {
    let actor = context.actor().clone();
    let old_enemy = context.state().old_enemy.clone();
    if old_enemy.as_ref().is_some_and(|old| health(&mut *context.game, Some(old)) > 0.0) {
        context.game.require_entity_mut(&actor).enemy = old_enemy;
        hunt_target(context);
    } else {
        let entity = context.game.require_entity_mut(&actor);
        entity.enemy = None;
        entity.goal = None;
        context.state_mut().old_enemy = None;
        if !context.find_target() {
            context.state_mut().pause_time = 100_000_000.0;
            context.stand();
            return false;
        }
    }
    true
}

/// Cleanup (`cleanup`).
fn medic_cleanup(context: &mut MonsterContext, change_frame: bool) {
    let actor = context.actor().clone();
    let target = context.game.require_entity(&actor).enemy.clone();
    if let Some(target) = target {
        cleanup_rogue_heal_target(&mut *context.game, &target);
    }
    if !medic_restore_enemy(context) {
        return;
    }
    if change_frame {
        context.state_mut().next_frame = medic_frame::ATTACK52;
    }
}

/// Abort (`abort`).
fn medic_abort(context: &mut MonsterContext, change_frame: bool, gib: bool, mark: bool) {
    let actor = context.actor().clone();
    let target = context.game.require_entity(&actor).enemy.clone();
    if let Some(target) = target.clone() {
        cleanup_rogue_heal_target(&mut *context.game, &target);
    }
    if target.is_some() && mark {
        let target = target.clone().expect("medic target");
        let bad1 = rogue_state(&mut *context.game, &target).bad_medic1.clone();
        let previous = bad1.as_ref().and_then(|bad| context.game.entity(bad).cloned());
        let rogue = rogue_state(&mut *context.game, &target);
        if previous.is_some_and(|previous| previous.classname.starts_with("monster_medic")) {
            rogue.bad_medic2 = Some(actor.clone());
        } else {
            rogue.bad_medic1 = Some(actor.clone());
        }
    }
    if target.is_some() && gib {
        let target = target.expect("medic target");
        let threshold = context
            .game
            .monsters
            .states
            .get(&target)
            .map(|state| state.gib_health)
            .unwrap_or(0.0);
        let origin = context.game.body_of(target.clone()).origin;
        context.game.damage(
            target,
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
    medic_cleanup(context, change_frame);
    context.state_mut().medic = false;
    rogue_state(&mut *context.game, &actor).medic_tries = 0;
}

/// Find dead (`findDead`).
fn medic_find_dead(context: &mut MonsterContext) -> Option<ActorId> {
    let actor = context.actor().clone();
    let react_to_damage_time = rogue_state(&mut *context.game, &actor).react_to_damage_time;
    if react_to_damage_time > context.game.host.now() {
        return None;
    }
    let origin = context.game.body_of(actor.clone()).origin;
    let radius = if context.state().stand_ground { 400.0 } else { 1024.0 };
    let nearby = context.game.host.nearby(origin, radius);
    let mut best: Option<ActorId> = None;
    let mut best_health = 0.0;
    for candidate in nearby {
        let target = context.game.entity(&candidate).cloned();
        let Some(target) = target else {
            continue;
        };
        let good_guy = context.game.monsters.states.get(&candidate).is_some_and(|state| state.good_guy);
        if candidate == actor
            || target.server_flags & 4 == 0
            || good_guy
            || target.classname.starts_with("player")
        {
            continue;
        }
        let rogue = rogue_state(&mut *context.game, &candidate);
        let (bad1, bad2, healer) = (rogue.bad_medic1.clone(), rogue.bad_medic2.clone(), rogue.healer.clone());
        if bad1 == Some(actor.clone()) || bad2 == Some(actor.clone()) {
            continue;
        }
        if let Some(healer) = healer {
            let healing = context.game.entity(&healer).is_some_and(|healer| healer.server_flags & 4 != 0)
                && health(&mut *context.game, Some(&healer)) > 0.0
                && context.game.monsters.states.get(&healer).is_some_and(|state| state.medic);
            if healing {
                continue;
            }
        }
        let dead_think = context.game.source_callbacks.resolve_think(Some("monster_dead_think"));
        if health(&mut *context.game, Some(&candidate)) > 0.0
            || target.next_think.is_some() && target.think != dead_think
            || !visible(context, Some(&candidate))
        {
            continue;
        }
        let distance = length3(sub3(origin, context.game.body_of(candidate.clone()).origin));
        if distance <= 32.0 {
            continue;
        }
        if best.is_none() || target.max_health > best_health {
            best_health = target.max_health;
            best = Some(candidate);
        }
    }
    if best.is_some() {
        let now = context.game.host.now();
        context.game.require_entity_mut(&actor).timestamp = now + 10.0;
    }
    best
}

/// Acquire (`acquire`).
fn medic_acquire(context: &mut MonsterContext) -> bool {
    let Some(target) = medic_find_dead(context) else {
        return false;
    };
    let actor = context.actor().clone();
    context.state_mut().old_enemy = context.game.require_entity(&actor).enemy.clone();
    context.game.require_entity_mut(&actor).enemy = Some(target.clone());
    rogue_state(&mut *context.game, &target).healer = Some(actor);
    context.state_mut().medic = true;
    found_target(context);
    true
}

/// Run (`run`).
fn rerelease_medic_run(context: &mut MonsterContext) {
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
        true,
    );
}

/// Idle (`idle`).
fn rerelease_medic_idle(context: &mut MonsterContext) {
    medic_sound(context, "idle", "medidle", 2, 2.0);
    if context.state().old_enemy.is_none() {
        medic_acquire(context);
    }
}

/// Attack (`attack`).
fn rerelease_medic_attack(context: &mut MonsterContext) {
    finish_dodge(context);
    let actor = context.actor().clone();
    let melee_range = target_distance(context) < 80.0;
    if rogue_state(&mut *context.game, &actor).blocked {
        context.set_move("medic_move_callReinforcements", true);
        rogue_state(&mut *context.game, &actor).blocked = false;
    }
    let random = context.game.host.random();
    let commander = monster_mass(context) > 400.0;
    let state = context.state();
    let (medic, slots, used, attack_state) =
        (state.medic, state.monster_slots, state.monster_used, state.attack_state);
    if medic {
        context.set_move(
            if commander && random > 0.8 && slots > used {
                "medic_move_callReinforcements"
            } else {
                "medic_move_attackCable"
            },
            true,
        );
        return;
    }
    context.set_move(
        if attack_state == MonsterAttackState::Blind
            || commander && random > 0.2 && !melee_range && slots > used
        {
            "medic_move_callReinforcements"
        } else {
            "medic_move_attackBlaster"
        },
        true,
    );
}

/// Attacking (`attacking`).
fn medic_attacking(context: &mut MonsterContext) -> bool {
    let current = context.state().current_move.name.clone();
    [
        "medic_move_attackHyperBlaster",
        "medic_move_attackCable",
        "medic_move_attackBlaster",
        "medic_move_callReinforcements",
    ]
    .contains(&current.as_str())
}

/// Cable (`cable`).
fn medic_cable(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let target = context.game.require_entity(&actor).enemy.clone();
    let target_body = target.as_ref().map(|target| context.game.body_of(target.clone()));
    let Some(target) = target else {
        return medic_abort(context, false, false, false);
    };
    if context.game.require_entity(&target).effects & 2 != 0 {
        return medic_abort(context, false, false, false);
    }
    if context.game.host.is_player(&target) {
        return;
    }
    if health(&mut *context.game, Some(&target)) > 0.0 {
        return medic_abort(context, false, false, false);
    }
    let target_body = target_body.expect("medic target body");
    let frame = context.game.require_entity(&actor).frame;
    let offset = *record_at(&CABLE_OFFSETS, (frame - medic_frame::ATTACK42) as usize);
    let start = project_flash(context, offset, None);
    if length3(sub3(start, target_body.origin)) < 32.0 {
        return medic_abort(context, true, true, false);
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
        && !matches!(&trace.hit, TraceHit::Actor { actor: hit } if hit == &target)
    {
        if source_trace_world(&mut *context.game, &trace) {
            if rogue_state(&mut *context.game, &actor).medic_tries > 1 {
                return medic_abort(context, true, false, true);
            }
            rogue_state(&mut *context.game, &actor).medic_tries += 1;
            return medic_cleanup(context, true);
        }
        return medic_abort(context, true, false, false);
    }
    if frame == medic_frame::ATTACK43 {
        let commander = monster_mass(context) > 400.0;
        let path = if commander {
            "medic_commander/medatck3a.wav"
        } else {
            "medic/medatck3.wav"
        };
        context.game.sound(&target, path, 0, 1.0, 1.0);
        if context.game.monsters.states.contains_key(&target) {
            {
                let mut patient = MonsterContext::new(target.clone(), &mut *context.game);
                patient.state_mut().resurrecting = true;
                patient.state_mut().can_take_damage = false;
                rogue_heal_effects(&mut patient);
            }
            let owned = context.game.owned_of(target.clone());
            context.game.host.combat().set_traits(
                &owned,
                &CombatTraitChanges {
                    can_take_damage: Some(false),
                    ..CombatTraitChanges::default()
                },
            );
        }
    } else if frame == medic_frame::ATTACK50 {
        {
            let target_entity = context.game.require_entity_mut(&target);
            target_entity.spawnflags = 0;
            target_entity.target = String::new();
            target_entity.targetname = String::new();
            target_entity.combat_target = String::new();
            target_entity.death_target = String::new();
            target_entity.health_target = String::new();
            target_entity.item_target = String::new();
        }
        if let Some(previous) = context.game.monsters.states.get_mut(&target) {
            previous.ignore_shots = false;
            previous.do_not_count = false;
            previous.good_guy = false;
            previous.target_anger = false;
            previous.brutal = false;
            previous.medic = false;
            previous.resurrecting = false;
            previous.stand_ground = false;
            previous.temporary_stand_ground = false;
            previous.hold_frame = false;
            previous.ducked = false;
            previous.dodging = false;
            previous.charging = false;
            previous.manual_steering = false;
            previous.combat_point = false;
            previous.lost_sight = false;
            previous.pursue_next = false;
            previous.pursue_temporary = false;
            previous.pursuit_last_seen = false;
            previous.sound_target = None;
        }
        rogue_state(&mut *context.game, &target).healer = Some(actor.clone());
        let mask = monster_solid_mask(&*context.game);
        let mut bounds = target_body.bounds;
        bounds.max.z += 48.0;
        let clear = context.game.host.trace(&Q2TraceRequest {
            start: target_body.origin,
            end: target_body.origin,
            bounds: Some(bounds),
            ignore: Some(target.clone()),
            mask,
            exclude: Vec::new(),
        });
        if clear.start_solid || clear.all_solid || !source_trace_world(&mut *context.game, &clear) {
            return medic_abort(context, true, true, false);
        }
        if let Some(previous) = context.game.monsters.states.get_mut(&target) {
            previous.do_not_count = true;
        }
        let max_health = context.game.require_entity(&target).max_health;
        let previous = context.game.monsters.states.get(&target).cloned();
        let gib_health = previous.as_ref().map(|state| state.gib_health).unwrap_or(0.0);
        let slots = previous.as_ref().map(|state| state.monster_slots).unwrap_or(0);
        let used = previous.as_ref().map(|state| state.monster_used).unwrap_or(0);
        let spawned_by = previous.as_ref().map(|state| state.spawned_by).unwrap_or_default();
        let commander = previous.as_ref().and_then(|state| state.commander.clone());
        let initial_power_armor = previous
            .as_ref()
            .map(|state| state.initial_power_armor)
            .unwrap_or_default();
        let max_power_armor_power =
            previous.as_ref().map(|state| state.max_power_armor_power).unwrap_or(0.0);
        let base_health = previous.as_ref().map(|state| state.base_health).unwrap_or(max_health);
        let health_scaling = previous.as_ref().map(|state| state.health_scaling).unwrap_or(1.0);
        respawn_monster(&mut *context.game, target.clone());
        if initial_power_armor == MonsterPowerArmor::None {
            let owned = context.game.owned_of(target.clone());
            context.game.host.combat().set_armor(
                &owned,
                &ArmorState {
                    regular: RegularArmorState::None,
                    powered: PoweredProtectionState::None,
                },
            );
            if context.game.host.inventory().has(&target) {
                let owned = context.game.owned_of(target.clone());
                context.game.host.inventory().configure(
                    &owned,
                    &InventoryEntry {
                        item: "q2:monster-power".to_string(),
                        count: max_power_armor_power,
                        capacity: max_power_armor_power,
                        count_policy: None,
                    },
                );
            }
        } else {
            let mut revived = MonsterContext::new(target.clone(), &mut *context.game);
            monster_power_armor(
                &mut revived,
                if initial_power_armor == MonsterPowerArmor::Shield {
                    PowerArmorKind::Shield
                } else {
                    PowerArmorKind::Screen
                },
                max_power_armor_power,
            );
        }
        {
            let mut revived = MonsterContext::new(target.clone(), &mut *context.game);
            revived.state_mut().initial_power_armor = initial_power_armor;
            revived.state_mut().max_power_armor_power = max_power_armor_power;
            revived.state_mut().base_health = base_health;
            revived.state_mut().health_scaling = health_scaling;
        }
        context.game.require_entity_mut(&target).max_health = max_health;
        let owned = context.game.owned_of(target.clone());
        context.game.host.combat().set_health(&owned, max_health);
        {
            let mut revived = MonsterContext::new(target.clone(), &mut *context.game);
            revived.state_mut().gib_health = (gib_health / 2.0).trunc();
            revived.state_mut().monster_slots = slots;
            revived.state_mut().monster_used = used;
            revived.state_mut().spawned_by = spawned_by;
            revived.state_mut().commander = commander;
        }
        let think = context.game.require_entity(&target).think;
        if let Some(think) = think {
            let now = context.game.host.now();
            context.game.require_entity_mut(&target).next_think = Some(now);
            think(target.clone(), &mut *context.game);
        }
        {
            let mut revived = MonsterContext::new(target.clone(), &mut *context.game);
            revived.state_mut().resurrecting = false;
            revived.state_mut().ignore_shots = true;
            revived.state_mut().do_not_count = true;
        }
        context.game.require_entity_mut(&target).effects &= !0x4000;
        rogue_state(&mut *context.game, &target).healer = None;
        let old_enemy = context.state().old_enemy.clone();
        let old_live = match old_enemy.as_ref() {
            Some(old) => {
                let live = context.game.host.actors().is_live(old);
                live && health(&mut *context.game, Some(old)) > 0.0
            }
            None => false,
        };
        if old_live {
            context.game.require_entity_mut(&target).enemy = old_enemy;
            let mut revived = MonsterContext::new(target, &mut *context.game);
            found_target(&mut revived);
        } else {
            context.game.require_entity_mut(&target).enemy = None;
            let mut revived = MonsterContext::new(target, &mut *context.game);
            if !revived.find_target() {
                let now = revived.game.host.now();
                revived.state_mut().pause_time = now + 100_000_000.0;
                revived.stand();
            }
            context.game.require_entity_mut(&actor).enemy = None;
            context.state_mut().old_enemy = None;
            if !context.find_target() {
                let now = context.game.host.now();
                context.state_mut().pause_time = now + 100_000_000.0;
                context.stand();
                return;
            }
        }
        medic_cleanup(context, false);
        return;
    } else if frame == medic_frame::ATTACK44 {
        medic_sound(context, "medatck4", "medatck4a", 1, 1.0);
    }
    let current_target = context.game.require_entity(&actor).enemy.clone();
    let Some(current_target) = current_target else {
        return;
    };
    let body = context.game.body_of(current_target);
    let origin = context.game.body_of(actor.clone()).origin;
    let forward = angles_vectors(context.game.body_of(actor.clone()).angles).forward;
    context.game.host_emit(Q2PresentationEvent::MonsterBeam {
        effect: Q2MonsterBeam::Medic,
        actor,
        start: add3(origin, scale3(forward, 8.0)),
        end: vec3(
            body.origin.x,
            body.origin.y,
            body.origin.z + (body.bounds.min.z + body.bounds.max.z) / 2.0,
        ),
    });
}

/// Reinforcement list (`reinforcementList`).
fn medic_reinforcement_list(context: &mut MonsterContext) -> Vec<Reinforcement> {
    let spawn = context.game.require_entity(context.actor()).spawn.clone();
    rerelease_medic_reinforcements(&spawn.values)
        .into_iter()
        .map(|entry| {
            let definition = monster_definition(&entry.classname, &*context.game)
                .unwrap_or_else(|| panic!("Unknown medic reinforcement {}", entry.classname));
            Reinforcement {
                classname: entry.classname,
                strength: entry.strength,
                bounds: definition.bounds.clone(),
            }
        })
        .collect()
}

/// Each spawn (`eachSpawn`).
fn medic_each_spawn(
    context: &mut MonsterContext,
    behind: bool,
    determine: bool,
    mut visit: impl FnMut(&mut MonsterContext, Vec3, &Reinforcement) -> bool,
) {
    let actor = context.actor().clone();
    let list = medic_reinforcement_list(context);
    let chosen = rogue_state(&mut *context.game, &actor).chosen_reinforcements.clone();
    for i in 0..chosen.len() {
        let reinforcement = record_at(&list, *record_at(&chosen, i) as usize);
        let mut position = *record_at(&REINFORCEMENT_POSITIONS, i);
        if determine {
            position = scale3(position, context.game.require_entity(&actor).scale as f32);
        }
        if behind {
            position = vec3(-position.x, -position.y, position.z);
        }
        let point = project_flash(context, position, None);
        let scale = context.game.require_entity(&actor).scale;
        let start = vec3(point.x, point.y, point.z + if behind { 10.0 } else { 10.0 * scale as f32 });
        let spawn = find_rerelease_spawn_point(
            &mut *context.game,
            start,
            reinforcement.bounds,
            32.0,
            true,
        );
        if let Some(spawn) = spawn {
            if visit(context, spawn, reinforcement) {
                break;
            }
        }
    }
}

/// Dead (`medic_dead`).
fn medic_dead(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let mut moved = context.game.body_of(actor.clone());
    moved.bounds.min = vec3(-16.0, -16.0, -24.0);
    moved.bounds.max = vec3(16.0, 16.0, -8.0);
    context.game.write_body(actor, &moved, true);
    corpse(context);
}

/// Shrink (`medic_shrink`).
fn medic_shrink(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let bounds = context.game.body_of(actor.clone()).bounds;
    context.game.require_entity_mut(&actor).server_flags |= 2;
    let mut moved = context.game.body_of(actor.clone());
    moved.bounds = bounds;
    moved.bounds.max.z = -2.0;
    context.game.write_body(actor, &moved, true);
}

/// Quick attack (`medic_quick_attack`).
fn medic_quick_attack(context: &mut MonsterContext) {
    if context.game.random() < 0.5 {
        context.set_move("medic_move_attackHyperBlaster", false);
        context.state_mut().next_frame = medic_frame::ATTACK16;
    }
}

/// Hook launch (`medic_hook_launch`).
fn medic_hook_launch(context: &mut MonsterContext) {
    medic_sound(context, "medatck2", "medatck2c", 1, 1.0);
}

/// Hook retract (`medic_hook_retract`).
fn medic_hook_retract(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "medic/medatck5.wav", 1, 1.0, 1.0);
    context.state_mut().medic = false;
    medic_restore_enemy(context);
}

/// Continue (`medic_continue`).
fn medic_continue(context: &mut MonsterContext) {
    if visible(context, None) && context.game.random() <= 0.95 {
        context.set_move("medic_move_attackHyperBlaster", false);
    }
}

/// Fire blaster (`medic_fire_blaster`).
fn medic_fire_blaster(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let frame = context.game.require_entity(&actor).frame;
    let commander = monster_mass(context) > 400.0;
    let blaster = frame == medic_frame::ATTACK9 || frame == medic_frame::ATTACK12;
    let flash = if blaster {
        if commander { 146 } else { 60 }
    } else {
        (if commander { 277 } else { 265 }) + frame - medic_frame::ATTACK19
    };
    let Some((start, direction)) = monster_shot(context, flash as usize, 0.0) else {
        return;
    };
    let effects = if blaster {
        8
    } else if frame % 4 == 0 {
        64
    } else {
        0
    };
    let enemy = context.entity().enemy.clone();
    let tesla = enemy.as_ref().and_then(|enemy| context.game.entity(enemy)).is_some_and(|target| {
        target.classname == "tesla_mine"
    });
    let damage = if tesla {
        3.0
    } else if blaster {
        6.0
    } else {
        2.0
    };
    if commander {
        let weapons = mission_weapons(&*context.game);
        weapons.fire_blaster2(actor, &mut *context.game, start, direction, damage, 1000.0, effects);
    } else {
        let fire_blaster = context.weapons.fire_blaster;
        fire_blaster(
            actor,
            &mut *context.game,
            start,
            direction,
            damage,
            1000.0,
            effects,
            false,
            crate::q2::foundation::weapons::types::Mod::BLASTER,
        );
    }
    monster_flash(context, flash, start, direction);
}

/// Start spawn (`medic_start_spawn`).
fn medic_start_spawn(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "medic_commander/monsterspawn1.wav", 1, 1.0, 1.0);
    context.state_mut().next_frame = medic_frame::ATTACK48;
}

/// Determine spawn (`medic_determine_spawn`).
fn medic_determine_spawn(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let list = medic_reinforcement_list(context);
    let count = 1.max((context.game.random() * 32.0).log2().trunc() as i32);
    let mut remaining = context.state().monster_slots - context.state().monster_used;
    let mut chosen = Vec::new();
    for _ in 0..count {
        if remaining == 0 {
            break;
        }
        let available: Vec<usize> = list
            .iter()
            .enumerate()
            .filter(|(_, reinforcement)| reinforcement.strength <= remaining)
            .map(|(index, _)| index)
            .collect();
        if available.is_empty() {
            break;
        }
        let index = *record_at(&available, rerelease_random(context).integer_max(available.len() as i32) as usize);
        chosen.push(index as i32);
        remaining -= record_at(&list, index).strength;
    }
    rogue_state(&mut *context.game, &actor).chosen_reinforcements = chosen;
    let mut success = false;
    medic_each_spawn(
        context,
        false,
        true,
        |context, point, reinforcement| {
            success = check_rerelease_ground_spawn_point(
                &mut *context.game,
                point,
                reinforcement.bounds,
                256.0,
                -1.0,
            );
            success
        },
    );
    if !success {
        medic_each_spawn(
            context,
            true,
            true,
            |context, point, reinforcement| {
                success = check_rerelease_ground_spawn_point(
                    &mut *context.game,
                    point,
                    reinforcement.bounds,
                    256.0,
                    -1.0,
                );
                success
            },
        );
        if success {
            context.state_mut().manual_steering = true;
            let yaw = medic_angle_mod(f64::from(context.game.body_of(actor.clone()).angles.y)) + 180.0;
            context.state_mut().ideal_yaw = if yaw > 360.0 { yaw - 360.0 } else { yaw };
        }
    }
    if !success {
        context.state_mut().next_frame = medic_frame::ATTACK53;
    }
}

/// Spawn grows (`medic_spawngrows`).
fn medic_spawngrows(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    if context.state().manual_steering {
        let yaw = medic_angle_mod(f64::from(context.game.body_of(actor).angles.y));
        if (yaw - context.state().ideal_yaw).abs() > 0.1 {
            context.state_mut().hold_frame = true;
            return;
        }
        context.state_mut().hold_frame = false;
        context.state_mut().manual_steering = false;
    }
    let mut success = false;
    medic_each_spawn(
        context,
        false,
        false,
        |context, point, reinforcement| {
            if check_rerelease_ground_spawn_point(
                &mut *context.game,
                point,
                reinforcement.bounds,
                256.0,
                -1.0,
            ) {
                success = true;
                medic_spawn_grow(
                    context,
                    add3(point, add3(reinforcement.bounds.min, reinforcement.bounds.max)),
                    f64::from(length3(sub3(reinforcement.bounds.max, reinforcement.bounds.min))) * 0.5,
                );
            }
            false
        },
    );
    if !success {
        context.state_mut().next_frame = medic_frame::ATTACK53;
    }
}

/// Finish spawn (`medic_finish_spawn`).
fn medic_finish_spawn(context: &mut MonsterContext) {
    medic_each_spawn(context, false, false, |context, point, reinforcement| {
        let actor = context.actor().clone();
        let bounds = reinforcement.bounds;
        if !check_rogue_spawn_point(&mut *context.game, point, bounds) {
            return false;
        }
        if !check_rerelease_ground_spawn_point(&mut *context.game, point, bounds, 256.0, -1.0) {
            return false;
        }
        let angles = context.game.body_of(actor.clone()).angles;
        let child = create_rogue_monster(&mut *context.game, point, angles, &reinforcement.classname);
        let think = context.game.require_entity(&child).think;
        if let Some(think) = think {
            let now = context.game.host.now();
            context.game.require_entity_mut(&child).next_think = Some(now);
            think(child.clone(), &mut *context.game);
        }
        if !context.game.monsters.states.contains_key(&child) {
            panic!("Medic reinforcement has no shared controller");
        }
        {
            let child_state = context.game.monsters.require_state_mut(&child);
            child_state.ignore_shots = true;
            child_state.do_not_count = true;
            child_state.spawned_by = MonsterSpawner::Medic;
            child_state.commander = Some(actor.clone());
            child_state.monster_slots = reinforcement.strength;
        }
        let used = context.state().monster_used;
        context.state_mut().monster_used = used + reinforcement.strength;
        let mut enemy = if context.state().medic {
            context.state().old_enemy.clone()
        } else {
            context.game.require_entity(&actor).enemy.clone()
        };
        let mut child_context = MonsterContext::new(child.clone(), &mut *context.game);
        if child_context.game.options.mode == crate::q2::foundation::host::Q2Mode::Coop {
            enemy = pick_rogue_coop_target(&mut child_context);
            let medic_enemy = child_context.game.require_entity(&actor).enemy.clone();
            if enemy == medic_enemy && enemy.is_some() {
                enemy = pick_rogue_coop_target(&mut child_context);
            }
            if enemy.is_none() {
                enemy = medic_enemy;
            }
        }
        let live = match enemy.as_ref() {
            Some(enemy) => {
                let live = child_context.game.host.actors().is_live(enemy);
                live && health(&mut *child_context.game, Some(enemy)) > 0.0
            }
            None => false,
        };
        if live {
            child_context.game.require_entity_mut(&child).enemy = enemy;
            found_target(&mut child_context);
        } else {
            child_context.game.require_entity_mut(&child).enemy = None;
            child_context.stand();
        }
        false
    });
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
fn rerelease_medic_initialize(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.state_mut().ignore_shots = true;
    if monster_mass(context) > 400.0 {
        context.game.require_entity_mut(&actor).skin = 2;
        let spawn = context.game.require_entity(&actor).spawn.clone();
        let slots = number_field(&spawn, "monster_slots", 3.0) as i32;
        context.state_mut().monster_slots = slots;
        if slots != 0 && !medic_reinforcement_list(context).is_empty() {
            let skill = f64::from(context.game.options.skill);
            let bonus = (f64::from(slots) * skill / 2.0).floor() as i32;
            context.state_mut().monster_slots = slots + bonus;
        }
    }
}

/// Pain (`pain`).
fn rerelease_medic_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    let actor = context.actor().clone();
    let commander = monster_mass(context) > 400.0;
    finish_dodge(context);
    let bloodied = health(&mut *context.game, Some(&actor))
        < context.game.require_entity(&actor).max_health / 2.0;
    let skin = context.game.require_entity(&actor).skin;
    context.game.require_entity_mut(&actor).skin = (skin & !1) | i32::from(bloodied);
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    let random = context.game.host.random();
    if commander {
        if reaction.damage < 35.0 {
            medic_sound(context, "medpain1", "medpain1", 2, 1.0);
            if !chainfist(context) {
                return;
            }
        }
        medic_sound(context, "medpain2", "medpain2", 2, 1.0);
    } else {
        medic_sound(
            context,
            if random < 0.5 { "medpain1" } else { "medpain2" },
            "medpain2",
            2,
            1.0,
        );
    }
    if !reacts_to_pain(context) || !chainfist(context) && context.state().medic {
        return;
    }
    if commander {
        context.state_mut().manual_steering = false;
        context.state_mut().hold_frame = false;
    }
    context.set_move(
        if commander {
            if random < (reaction.damage * 0.005).min(0.5) {
                "medic_move_pain2"
            } else {
                "medic_move_pain1"
            }
        } else if random < 0.5 {
            "medic_move_pain1"
        } else {
            "medic_move_pain2"
        },
        true,
    );
    if context.state().ducked {
        set_duck(context, false);
    }
    medic_abort(context, false, false, false);
}

/// Die (`die`).
fn rerelease_medic_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    let actor = context.actor().clone();
    if context.state().medic {
        let target = context.game.require_entity(&actor).enemy.clone();
        if let Some(target) = target {
            cleanup_rogue_heal_target(&mut *context.game, &target);
        }
        context.state_mut().medic = false;
    }
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
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/objects/gibs/sm_meat/tris.md2",
            damage,
            Q2GibOptions::default(),
        );
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/objects/gibs/sm_metal/tris.md2",
            damage,
            Q2GibOptions {
                metallic: true,
                ..Q2GibOptions::default()
            },
        );
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/monsters/medic/gibs/chest.md2",
            damage,
            Q2GibOptions {
                skinned: true,
                ..Q2GibOptions::default()
            },
        );
        for part in ["leg", "leg", "hook", "gun"] {
            throw_gib(
                actor.clone(),
                &mut *context.game,
                &format!("models/monsters/medic/gibs/{part}.md2"),
                damage,
                Q2GibOptions {
                    skinned: true,
                    upright: true,
                    ..Q2GibOptions::default()
                },
            );
        }
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/monsters/medic/gibs/head.md2",
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
    medic_sound(context, "meddeth1", "meddeth", 2, 1.0);
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
    context.set_move("medic_move_death", true);
}

/// Duck (`duck`).
fn rerelease_medic_duck(context: &mut MonsterContext, _eta: f64) -> bool {
    if context.state().medic {
        return false;
    }
    if medic_attacking(context) {
        set_duck(context, false);
        return false;
    }
    context.set_move("medic_move_duck", true);
    true
}

/// Sidestep (`sidestep`).
fn rerelease_medic_sidestep(context: &mut MonsterContext) -> bool {
    if medic_attacking(context) {
        return false;
    }
    if context.state().current_move.name != "medic_move_run" {
        context.set_move("medic_move_run", true);
    }
    true
}

/// Check attack (`checkAttack`).
fn rerelease_medic_check_attack(context: &mut MonsterContext) -> bool {
    let actor = context.actor().clone();
    if context.state().medic {
        let enemy = context.game.require_entity(&actor).enemy.clone();
        let lost = match enemy.as_ref() {
            None => true,
            Some(enemy) => !context.game.host.actors().is_live(enemy),
        };
        if lost {
            medic_abort(context, true, false, false);
            return false;
        }
        if context.game.require_entity(&actor).timestamp < context.game.host.now() {
            medic_abort(context, true, false, true);
            context.game.require_entity_mut(&actor).timestamp = 0.0;
            return false;
        }
        if target_distance(context) < 410.0 {
            rerelease_medic_attack(context);
            return true;
        }
        context.state_mut().attack_state = MonsterAttackState::Straight;
        return false;
    }
    let enemy = context.game.require_entity(&actor).enemy.clone();
    if enemy.as_ref().is_some_and(|enemy| context.game.host.is_player(enemy))
        && !visible(context, None)
        && context.state().monster_slots > context.state().monster_used
    {
        context.state_mut().attack_state = MonsterAttackState::Blind;
        return true;
    }
    let (slots, used) = (context.state().monster_slots, context.state().monster_used);
    if slots != 0
        && context.game.random() < 0.8
        && f64::from(slots - used) > f64::from(slots) * 0.8
        && target_distance(context) > 150.0
    {
        rogue_state(&mut *context.game, &actor).blocked = true;
        context.state_mut().attack_state = MonsterAttackState::Missile;
        return true;
    }
    if context.state().stand_ground {
        context.state_mut().attack_state = MonsterAttackState::Missile;
        return true;
    }
    default_check_attack(context)
}

/// Create the rerelease medic definitions (`createRereleaseMedicDefinitions`).
pub fn create_rerelease_medic_definitions() -> Vec<Q2MonsterDefinition> {
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
        MonsterHandler::Callback(rerelease_medic_run),
        MonsterHandler::Callback(rerelease_medic_attack),
        rerelease_medic_die,
    );
    definition.idle = Some(MonsterHandler::Callback(rerelease_medic_idle));
    let mut source_callbacks = Q2CallbackDefinitions::default();
    source_callbacks.think.insert("rerelease.medic.spawngrow_think", spawn_grow_think);
    source_callbacks.think.insert("rerelease.medic.SpawnGro_laser_think", spawn_grow_laser_think);
    definition.source_callbacks = Some(source_callbacks);
    definition.sight = Some(MonsterHandler::Callback(medic_sight));
    definition.search = Some(MonsterHandler::Callback(medic_search));
    definition.initialize = Some(MonsterHandler::Callback(rerelease_medic_initialize));
    definition.pain = Some(rerelease_medic_pain);
    definition.duck = Some(rerelease_medic_duck);
    definition.sidestep = Some(rerelease_medic_sidestep);
    definition.blocked = Some(blocked_check_platform);
    definition.check_attack = Some(rerelease_medic_check_attack);
    for (name, handler) in [
        ("medic_idle", MonsterHandler::Callback(rerelease_medic_idle)),
        ("medic_run", MonsterHandler::Callback(rerelease_medic_run)),
        ("medic_dead", MonsterHandler::Callback(medic_dead)),
        ("medic_shrink", MonsterHandler::Callback(medic_shrink)),
        (
            "medic_quick_attack",
            MonsterHandler::Callback(medic_quick_attack),
        ),
        (
            "monster_done_dodge",
            MonsterHandler::Callback(finish_dodge),
        ),
        ("medic_hook_launch", MonsterHandler::Callback(medic_hook_launch)),
        (
            "medic_hook_retract",
            MonsterHandler::Callback(medic_hook_retract),
        ),
        ("medic_cable_attack", MonsterHandler::Callback(medic_cable)),
        ("medic_continue", MonsterHandler::Callback(medic_continue)),
        (
            "medic_fire_blaster",
            MonsterHandler::Callback(medic_fire_blaster),
        ),
        ("medic_start_spawn", MonsterHandler::Callback(medic_start_spawn)),
        (
            "medic_determine_spawn",
            MonsterHandler::Callback(medic_determine_spawn),
        ),
        ("medic_spawngrows", MonsterHandler::Callback(medic_spawngrows)),
        (
            "medic_finish_spawn",
            MonsterHandler::Callback(medic_finish_spawn),
        ),
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
