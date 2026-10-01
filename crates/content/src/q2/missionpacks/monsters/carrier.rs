//! Rogue carrier (`src/content/q2/missionpacks/monsters/carrier.ts`).
//!
//! Quake II rogue/m_carrier.c. ZeniMax Media, GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::{add3, dot3, normalize3, scale3, sub3, vec3, Bounds, Vec3};

use super::spawn::{create_rogue_monster, find_rogue_spawn_point, rogue_spawn_callbacks, rogue_spawn_grow};
use super::tables::rogue_carrier::{carrier_frame, carrier_moves};
use super::types::mission_services;
use crate::q2::base::monsters::boss_common::{boss_explode, with_boss_explosion_callbacks};
use crate::q2::base::monsters::common::{damaged_skin, finish_corpse, monster_loop_sound, move_handler, sound_handler};
use crate::q2::foundation::host::{Q2Mode, Q2TraceRequest};
use crate::q2::foundation::monsters::ai::{
    angles_vectors, enemy_body, enemy_eye, health, in_front, project_flash, target_distance, vector_angles,
};
use crate::q2::foundation::monsters::muzzle::muzzle_offset;
use crate::q2::foundation::monsters::perception::found_target;
use crate::q2::foundation::monsters::types::{
    record_at, MonsterAttackState, MonsterContext, MonsterHandler, MonsterLocomotion, MonsterSpawner,
    Q2MonsterDefinition,
};
use crate::q2::rerelease::monsters::common::{monster_flash, predicted_direction};
use crate::q2::support::contracts::{CombatTraitChanges, DeathReaction, PainReaction, TraceHit};

/// Flyer bounds (`flyerBounds`).
const FLYER_BOUNDS: Bounds = Bounds {
    min: Vec3 {
        x: -16.0,
        y: -16.0,
        z: -24.0,
    },
    max: Vec3 {
        x: 16.0,
        y: 16.0,
        z: 16.0,
    },
};

/// Enemy relation (`relation`).
struct CarrierRelation {
    /// In front.
    front: bool,
    /// Behind.
    back: bool,
    /// Below.
    below: bool,
}

/// Enemy relation (`relation`).
fn carrier_relation(context: &mut MonsterContext, actor: &ActorId) -> CarrierRelation {
    let none = CarrierRelation {
        front: false,
        back: false,
        below: false,
    };
    let Some(target) = context.game.host.bodies().read(actor) else {
        return none;
    };
    let self_actor = context.actor().clone();
    let body = context.game.body_of(self_actor);
    let direction = normalize3(sub3(target.origin, body.origin));
    let forward = dot3(direction, angles_vectors(body.angles).forward);
    CarrierRelation {
        front: forward > 0.3,
        back: forward < -0.3,
        below: -direction.z > 0.95,
    }
}

/// Angle mod (`anglemod`).
fn angle_mod(angle: f64) -> f64 {
    ((angle * 65536.0 / 360.0).trunc() as i64 & 65535) as f64 * 360.0 / 65536.0
}

/// Rocket (`rocket`).
fn carrier_rocket(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let enemy_id = context.entity().enemy.clone();
    let Some(enemy) = enemy_body(context) else {
        return;
    };
    let Some(enemy_id) = enemy_id else {
        return;
    };
    let predictive = context.game.host.is_player(&enemy_id) && context.game.random() < 0.5;
    let body = context.game.body_of(actor.clone());
    let right = angles_vectors(body.angles).right;
    for index in 0..4 {
        let flash = 191 + index;
        let edition = context.game.options.edition;
        let start = project_flash(context, muzzle_offset(edition, flash), None);
        let spread = *record_at(&[0.4, 0.025, -0.025, -0.4], index);
        let direction = if predictive {
            predicted_direction(context, start, 750.0, false, -0.3 + index as f64 * 0.15)
        } else {
            let aimed = vec3(
                enemy.origin.x,
                enemy.origin.y,
                enemy.origin.z - if index == 0 || index == 3 { 15.0 } else { 0.0 },
            );
            Some(normalize3(add3(
                normalize3(sub3(aimed, start)),
                scale3(right, spread as f32),
            )))
        };
        let Some(direction) = direction else {
            continue;
        };
        let fire_rocket = context.weapons.fire_rocket;
        fire_rocket(
            actor.clone(),
            &mut *context.game,
            start,
            direction,
            50.0,
            if predictive { 750.0 } else { 500.0 },
            70.0,
            50.0,
        );
        monster_flash(context, flash as i32, start, direction);
    }
}

/// Coop check (`coopCheck`).
fn carrier_coop_check(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    if context.game.options.mode != Q2Mode::Coop || context.entity().wait > context.game.host.now() {
        return;
    }
    let mut targets = Vec::new();
    let players = context.game.host.players();
    for player in players {
        if !context.game.host.actors().is_live(&player) || !context.game.host.is_player(&player) {
            continue;
        }
        let direction = carrier_relation(context, &player);
        let target = context.game.host.bodies().read(&player);
        let Some(target) = target else {
            continue;
        };
        let origin = context.game.body_of(actor.clone()).origin;
        let trace = context.game.host.trace(&Q2TraceRequest {
            start: origin,
            end: target.origin,
            bounds: None,
            ignore: Some(actor.clone()),
            mask: 3,
            exclude: Vec::new(),
        });
        if (direction.back || direction.below) && trace.fraction == 1.0 {
            targets.push(player);
        }
    }
    if targets.is_empty() {
        return;
    }
    let pick = (context.game.random() * targets.len() as f64).floor() as usize;
    let chosen = record_at(&targets, pick.min(targets.len() - 1)).clone();
    let wait = context.game.host.now() + 2.0;
    context.entity_mut().wait = wait;
    let previous = context.entity().enemy.clone();
    context.entity_mut().enemy = Some(chosen);
    carrier_rocket(context);
    context.entity_mut().enemy = previous;
}

/// Machine gun (`machineGun`).
fn carrier_machine_gun(context: &mut MonsterContext) {
    carrier_coop_check(context);
    for right_gun in [false, true] {
        let enemy = enemy_body(context);
        let eye = enemy_eye(context);
        let (Some(enemy), Some(eye)) = (enemy, eye) else {
            continue;
        };
        let base = if context.state().manual_steering { 152 } else { 138 };
        let flash = base + if right_gun { 1 } else { 0 };
        let edition = context.game.options.edition;
        let start = project_flash(context, muzzle_offset(edition, flash as usize), None);
        let direction = normalize3(sub3(
            add3(eye, scale3(enemy.velocity, if right_gun { 0.2 } else { -0.2 })),
            start,
        ));
        let fire_bullet = context.weapons.fire_bullet;
        let actor = context.actor().clone();
        fire_bullet(actor, &mut *context.game, start, direction, 6.0, 4.0, 900.0, 500.0, 0);
        monster_flash(context, flash, start, direction);
    }
}

/// Spawn (`spawn`).
fn carrier_spawn(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let start = project_flash(context, vec3(105.0, 0.0, -58.0), None);
    let point = find_rogue_spawn_point(&mut *context.game, start, FLYER_BOUNDS, 32.0);
    let time = ((context.game.host.now() + 0.1 - context.entity().timestamp) / 0.5).trunc() as i32;
    let Some(point) = point else {
        return;
    };
    let body_angles = context.game.body_of(actor.clone()).angles;
    let child = create_rogue_monster(
        &mut *context.game,
        point,
        body_angles,
        if time == 2 { "monster_kamikaze" } else { "monster_flyer" },
    );
    context
        .game
        .sound(&actor, "medic_commander/monsterspawn1.wav", 4, 1.0, 0.0);
    context.state_mut().monster_slots -= 1;
    let now = context.game.host.now();
    context.game.require_entity_mut(&child).next_think = Some(now);
    let think = context.game.require_entity(&child).think;
    if let Some(think) = think {
        think(child.clone(), &mut *context.game);
    }
    if !context.game.monsters.states.contains_key(&child) {
        panic!("Carrier child has no source controller");
    }
    if let Some(state) = context.game.monsters.states.get_mut(&child) {
        state.spawned_by = MonsterSpawner::Carrier;
        state.do_not_count = true;
        state.ignore_shots = true;
        state.commander = Some(actor.clone());
    }
    let enemy = context.entity().enemy.clone();
    let spawn = enemy.as_ref().is_some_and(|enemy| {
        context.game.host.actors().is_live(enemy) && health(&mut *context.game, Some(enemy)) > 0.0
    });
    if !spawn {
        return;
    }
    let enemy = enemy.expect("carrier spawn enemy");
    context.game.require_entity_mut(&child).enemy = Some(enemy);
    let mut child_context = MonsterContext::new(child.clone(), &mut *context.game);
    found_target(&mut child_context);
    if time == 1 || time == 3 {
        child_context.state_mut().lefty = time == 3;
        child_context.state_mut().attack_state = MonsterAttackState::Sliding;
        child_context.set_move("flyer_move_attack3", true);
    } else if time == 2 {
        child_context.state_mut().lefty = false;
        child_context.state_mut().attack_state = MonsterAttackState::Straight;
        child_context.set_move("flyer_move_kamikaze", true);
        let owned = child_context.game.owned_of(child.clone());
        child_context.game.set_combat_traits(
            &owned,
            &CombatTraitChanges {
                mass: Some(100.0),
                ..CombatTraitChanges::default()
            },
        );
        child_context.state_mut().charging = true;
    }
}

/// Run (`run`).
fn carrier_run(context: &mut MonsterContext) {
    context.state_mut().hold_frame = false;
    let stand_ground = context.state().stand_ground;
    context.set_move(
        if stand_ground {
            "carrier_move_stand"
        } else {
            "carrier_move_run"
        },
        false,
    );
}

/// Rail move (`railMove`).
fn carrier_rail_move(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "gladiator/railgun.wav", 1, 1.0, 1.0);
    context.set_move("carrier_move_attack_rail", true);
}

/// Initialize (`initialize`).
fn carrier_initialize(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let skill = context.game.options.skill;
    let mut hp = (2000.0 + 1000.0 * f64::from(skill - 1)).max(2000.0);
    if context.game.options.mode == Q2Mode::Coop {
        hp += 500.0 * f64::from(skill);
    }
    context.entity_mut().max_health = hp;
    let owned = context.game.owned_of(actor);
    context.game.host.combat().set_health(&owned, hp);
    context.entity_mut().laser_immune = true;
    context.state_mut().ignore_shots = true;
    context.state_mut().monster_slots = if skill == 0 {
        3
    } else if skill == 3 {
        9
    } else {
        6
    };
    monster_loop_sound(context, "bosshovr/bhvengn1.wav");
}

/// Attack (`attack`).
fn carrier_attack(context: &mut MonsterContext) {
    context.state_mut().hold_frame = false;
    let actor = context.actor().clone();
    let enemy = context.entity().enemy.clone();
    let Some(enemy_id) = enemy else {
        return;
    };
    if !context.game.host.actors().is_live(&enemy_id) {
        return;
    }
    let relation = carrier_relation(context, &enemy_id);
    let ready = context.game.host.now() >= context.state().attack_finished;
    if mission_services(&*context.game).bad_area(&actor) {
        if relation.back || relation.below {
            context.set_move("carrier_move_attack_rocket", true);
            return;
        }
        if context.game.random() < 0.1 || !ready {
            context.set_move("carrier_move_attack_pre_mg", true);
        } else {
            carrier_rail_move(context);
        }
        return;
    }
    if context.state().attack_state == MonsterAttackState::Blind {
        context.set_move("carrier_move_spawn", true);
        return;
    }
    if !relation.back && !relation.front && !relation.below {
        if context.game.random() < 0.1 || !ready {
            context.set_move("carrier_move_attack_pre_mg", true);
        } else {
            carrier_rail_move(context);
        }
        return;
    }
    if relation.front {
        let distance = target_distance(context);
        if distance <= 125.0 {
            if context.game.random() < 0.8 || !ready {
                context.set_move("carrier_move_attack_pre_mg", true);
            } else {
                carrier_rail_move(context);
            }
            return;
        }
        let luck = context.game.random();
        let slots = context.state().monster_slots;
        if distance < 600.0 {
            if slots > 2 {
                if luck <= 0.2 {
                    context.set_move("carrier_move_attack_pre_mg", true);
                } else if luck <= 0.4 {
                    context.set_move("carrier_move_attack_pre_gren", true);
                } else if luck <= 0.7 && ready {
                    carrier_rail_move(context);
                } else {
                    context.set_move("carrier_move_spawn", true);
                }
                return;
            }
            if luck <= 0.3 {
                context.set_move("carrier_move_attack_pre_mg", true);
            } else if luck <= 0.65 {
                context.set_move("carrier_move_attack_pre_gren", true);
            } else if ready {
                carrier_rail_move(context);
            } else {
                context.set_move("carrier_move_attack_pre_mg", true);
            }
            return;
        }
        if slots > 2 {
            if luck < 0.3 {
                context.set_move("carrier_move_attack_pre_mg", true);
            } else if luck < 0.65 && ready {
                if let Some(eye) = enemy_eye(context) {
                    context.entity_mut().pos1 = eye;
                }
                carrier_rail_move(context);
            } else {
                context.set_move("carrier_move_spawn", true);
            }
            return;
        }
        if luck < 0.45 || !ready {
            context.set_move("carrier_move_attack_pre_mg", true);
        } else {
            carrier_rail_move(context);
        }
        return;
    }
    if relation.below || relation.back {
        context.set_move("carrier_move_attack_rocket", true);
    }
}

/// Check attack (`checkAttack`).
fn carrier_check_attack(context: &mut MonsterContext) -> bool {
    let actor = context.actor().clone();
    let enemy = context.entity().enemy.clone();
    let enemy_state = enemy_body(context);
    let eye = enemy_eye(context);
    let (Some(enemy_id), Some(enemy_state), Some(eye)) = (enemy, enemy_state, eye) else {
        return false;
    };
    if health(&mut *context.game, Some(&enemy_id)) > 0.0 {
        let origin = context.game.body_of(actor.clone()).origin;
        let start = vec3(origin.x, origin.y, origin.z + context.entity().view_height as f32);
        let trace = context.game.host.trace(&Q2TraceRequest {
            start,
            end: eye,
            bounds: None,
            ignore: Some(actor.clone()),
            mask: 1 | 0x2000000 | 8 | 16,
            exclude: Vec::new(),
        });
        if !matches!(&trace.hit, TraceHit::Actor { actor } if *actor == enemy_id) {
            if context.game.host.is_player(&enemy_id) && context.state().monster_slots > 2 {
                context.state_mut().attack_state = MonsterAttackState::Blind;
                return true;
            }
            let solid = context.game.entity(&enemy_id).map(|enemy| enemy.solid);
            if solid != Some(crate::q2::foundation::host::Q2Solid::None) || trace.fraction < 1.0 {
                return false;
            }
        }
    }
    let direction = carrier_relation(context, &enemy_id);
    let distance = target_distance(context);
    let enemy_origin = enemy_state.origin;
    let origin = context.game.body_of(actor.clone()).origin;
    context.state_mut().ideal_yaw = f64::from(vector_angles(sub3(enemy_origin, origin)).y);
    if (direction.back || !direction.front && direction.below) && context.game.host.now() >= context.entity().wait {
        let wait = context.game.host.now() + 2.0;
        context.entity_mut().wait = wait;
        context.attack();
        let sliding = context.game.random() < 0.6;
        context.state_mut().attack_state = if sliding {
            MonsterAttackState::Sliding
        } else {
            MonsterAttackState::Straight
        };
        return true;
    }
    if distance < 80.0 {
        context.state_mut().attack_state = MonsterAttackState::Missile;
        return true;
    }
    let solid_none = context
        .game
        .entity(&enemy_id)
        .is_some_and(|enemy| enemy.solid == crate::q2::foundation::host::Q2Solid::None);
    let stand_ground = context.state().stand_ground;
    let chance = if stand_ground {
        0.4
    } else if distance < 1000.0 {
        0.8
    } else {
        0.5
    };
    if context.game.random() < chance || solid_none {
        context.state_mut().attack_state = MonsterAttackState::Missile;
        return true;
    }
    if context.entity().flags & 1 != 0 {
        let sliding = context.game.random() < 0.6;
        context.state_mut().attack_state = if sliding {
            MonsterAttackState::Sliding
        } else {
            MonsterAttackState::Straight
        };
    }
    false
}

/// Pain (`pain`).
fn carrier_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    damaged_skin(context);
    if context.game.options.skill == 3 || context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 5.0;
    let mut changed = false;
    let actor = context.actor().clone();
    if reaction.damage < 10.0 {
        context.game.sound(&actor, "carrier/pain_sm.wav", 2, 1.0, 0.0);
    } else if reaction.damage < 30.0 {
        context.game.sound(&actor, "carrier/pain_md.wav", 2, 1.0, 0.0);
        if context.game.random() < 0.5 {
            changed = true;
            context.set_move("carrier_move_pain_light", true);
        }
    } else {
        context.game.sound(&actor, "carrier/pain_lg.wav", 2, 1.0, 0.0);
        context.set_move("carrier_move_pain_heavy", true);
        changed = true;
    }
    if changed {
        context.state_mut().hold_frame = false;
        context.state_mut().manual_steering = false;
        context.state_mut().yaw_speed = 15.0;
    }
}

/// Die (`die`).
fn carrier_die(context: &mut MonsterContext, _reaction: &DeathReaction) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "carrier/death.wav", 2, 1.0, 0.0);
    context.state_mut().dead = true;
    context.state_mut().can_take_damage = false;
    let owned = context.game.owned_of(actor);
    context.game.set_combat_traits(
        &owned,
        &CombatTraitChanges {
            can_take_damage: Some(false),
            ..CombatTraitChanges::default()
        },
    );
    context.entity_mut().count = 0;
    context.set_move("carrier_move_death", true);
}

/// Dead (`carrier_dead`).
fn carrier_dead(context: &mut MonsterContext) {
    finish_corpse(
        context,
        Bounds {
            min: vec3(-56.0, -56.0, 0.0),
            max: vec3(56.0, 56.0, 80.0),
        },
    );
}

/// Grenade (`CarrierGrenade`).
fn carrier_grenade(context: &mut MonsterContext) {
    carrier_coop_check(context);
    let Some(enemy) = enemy_body(context) else {
        return;
    };
    let actor = context.actor().clone();
    let direction = if context.game.random() < 0.5 { -1.0 } else { 1.0 };
    let time = ((context.game.host.now() - context.entity().timestamp) / 0.4).trunc() as i32;
    let right_spread = if time == 0 {
        0.15 * direction
    } else if time == 2 {
        -0.15 * direction
    } else {
        0.0
    };
    let up_spread = if time == 0 {
        0.1 - 0.1 * direction
    } else if time == 2 {
        0.1 + 0.1 * direction
    } else if time == 1 || time == 3 {
        0.1
    } else {
        0.0
    };
    let edition = context.game.options.edition;
    let start = project_flash(context, muzzle_offset(edition, 140), None);
    let body = context.game.body_of(actor);
    let axes = angles_vectors(body.angles);
    let aim = add3(
        add3(
            normalize3(sub3(enemy.origin, start)),
            scale3(axes.right, right_spread as f32),
        ),
        scale3(axes.up, up_spread as f32),
    );
    let clipped = vec3(aim.x, aim.y, aim.z.clamp(-0.5, 0.15));
    let fire_grenade = context.weapons.fire_grenade;
    let actor = context.actor().clone();
    fire_grenade(
        actor,
        &mut *context.game,
        start,
        clipped,
        50.0,
        600.0,
        2.5,
        90.0,
        false,
        false,
        true,
        None,
    );
    monster_flash(context, 53, start, clipped);
}

/// Save loc (`CarrierSaveLoc`).
fn carrier_save_loc(context: &mut MonsterContext) {
    carrier_coop_check(context);
    if let Some(eye) = enemy_eye(context) {
        context.entity_mut().pos1 = eye;
    }
}

/// Rail (`CarrierRail`).
fn carrier_rail(context: &mut MonsterContext) {
    carrier_coop_check(context);
    let edition = context.game.options.edition;
    let start = project_flash(context, muzzle_offset(edition, 147), None);
    let pos1 = context.entity().pos1;
    let direction = normalize3(sub3(pos1, start));
    let fire_rail = context.weapons.fire_rail;
    let actor = context.actor().clone();
    fire_rail(actor, &mut *context.game, start, direction, 50.0, 100.0);
    monster_flash(context, 147, start, direction);
    let finished = context.game.host.now() + 3.0;
    context.state_mut().attack_finished = finished;
}

/// Attack MG (`carrier_attack_mg`).
fn carrier_attack_mg(context: &mut MonsterContext) {
    carrier_coop_check(context);
    context.set_move("carrier_move_attack_mg", true);
}

/// Reattack MG (`carrier_reattack_mg`).
fn carrier_reattack_mg(context: &mut MonsterContext) {
    carrier_coop_check(context);
    let enemy = context.entity().enemy.clone();
    let gate = match enemy {
        Some(enemy) => in_front(context, &enemy) && context.game.random() <= 0.5,
        None => false,
    };
    // The donor draws its second random only when the first gate passes.
    let moves = if gate {
        let attack = context.game.random() < 0.7 || context.state().monster_slots <= 2;
        if attack {
            "carrier_move_attack_mg"
        } else {
            "carrier_move_spawn"
        }
    } else {
        "carrier_move_attack_post_mg"
    };
    context.set_move(moves, true);
}

/// Attack gren (`carrier_attack_gren`).
fn carrier_attack_gren(context: &mut MonsterContext) {
    carrier_coop_check(context);
    let now = context.game.host.now();
    context.entity_mut().timestamp = now;
    context.set_move("carrier_move_attack_gren", true);
}

/// Reattack gren (`carrier_reattack_gren`).
fn carrier_reattack_gren(context: &mut MonsterContext) {
    carrier_coop_check(context);
    let enemy = context.entity().enemy.clone();
    let again = match enemy {
        Some(enemy) => in_front(context, &enemy) && context.entity().timestamp + 1.3 > context.game.host.now(),
        None => false,
    };
    context.set_move(
        if again {
            "carrier_move_attack_gren"
        } else {
            "carrier_move_attack_post_gren"
        },
        false,
    );
}

/// Prep spawn (`carrier_prep_spawn`).
fn carrier_prep_spawn(context: &mut MonsterContext) {
    carrier_coop_check(context);
    context.state_mut().manual_steering = true;
    let now = context.game.host.now();
    context.entity_mut().timestamp = now;
    context.state_mut().yaw_speed = 10.0;
    carrier_machine_gun(context);
}

/// Start spawn (`carrier_start_spawn`).
fn carrier_start_spawn(context: &mut MonsterContext) {
    carrier_coop_check(context);
    let Some(enemy) = enemy_body(context) else {
        return;
    };
    let actor = context.actor().clone();
    let time = ((context.game.host.now() - context.entity().timestamp) / 0.5).trunc() as i32;
    let origin = context.game.body_of(actor).origin;
    let yaw = vector_angles(sub3(enemy.origin, origin)).y;
    if (0..=2).contains(&time) {
        context.state_mut().ideal_yaw = angle_mod(f64::from(yaw) + f64::from(time - 1) * 30.0);
    }
    carrier_machine_gun(context);
}

/// Ready spawn (`carrier_ready_spawn`).
fn carrier_ready_spawn(context: &mut MonsterContext) {
    carrier_coop_check(context);
    carrier_machine_gun(context);
    let actor = context.actor().clone();
    let yaw = context.game.body_of(actor).angles.y;
    if (angle_mod(f64::from(yaw)) - context.state().ideal_yaw).abs() > 0.1 {
        context.state_mut().hold_frame = true;
        context.entity_mut().timestamp += 0.1;
        return;
    }
    context.state_mut().hold_frame = false;
    let start = project_flash(context, vec3(105.0, 0.0, -58.0), None);
    let point = find_rogue_spawn_point(&mut *context.game, start, FLYER_BOUNDS, 32.0);
    if let Some(point) = point {
        rogue_spawn_grow(&mut *context.game, point, 0);
    }
}

/// Spawn check (`carrier_spawn_check`).
fn carrier_spawn_check(context: &mut MonsterContext) {
    carrier_coop_check(context);
    carrier_machine_gun(context);
    carrier_spawn(context);
    if context.game.host.now() > context.entity().timestamp + 1.1 {
        context.state_mut().manual_steering = false;
        context.state_mut().yaw_speed = 15.0;
    } else {
        context.state_mut().next_frame = carrier_frame::SPAWN08;
    }
}

/// Create the carrier definition (`createCarrierDefinition`).
pub fn create_carrier_definition() -> Q2MonsterDefinition {
    let mut definition = Q2MonsterDefinition::new(
        "monster_carrier",
        "carrier",
        "models/monsters/carrier/tris.md2",
        2000.0,
        -200.0,
        1000.0,
        Bounds {
            min: vec3(-56.0, -56.0, -44.0),
            max: vec3(56.0, 56.0, 44.0),
        },
        1.0,
        "carrier_move_stand",
        carrier_moves(),
        move_handler("carrier_move_stand"),
        move_handler("carrier_move_walk"),
        MonsterHandler::Callback(carrier_run),
        MonsterHandler::Callback(carrier_attack),
        carrier_die,
    );
    definition.yaw_speed = Some(15.0);
    definition.locomotion = Some(MonsterLocomotion::Fly);
    definition.sight = Some(sound_handler("carrier/sight.wav", 2, 1.0));
    definition.source_callbacks = Some(rogue_spawn_callbacks());
    definition.initialize = Some(MonsterHandler::Callback(carrier_initialize));
    definition.check_attack = Some(carrier_check_attack);
    definition.pain = Some(carrier_pain);
    for (name, handler) in [
        ("carrier_run", MonsterHandler::Callback(carrier_run)),
        ("carrier_dead", MonsterHandler::Callback(carrier_dead)),
        ("BossExplode", MonsterHandler::Callback(boss_explode)),
        ("CarrierCoopCheck", MonsterHandler::Callback(carrier_coop_check)),
        ("CarrierMachineGun", MonsterHandler::Callback(carrier_machine_gun)),
        ("CarrierMachineGunHold", MonsterHandler::Callback(carrier_machine_gun)),
        ("CarrierRocket", MonsterHandler::Callback(carrier_rocket)),
        ("CarrierGrenade", MonsterHandler::Callback(carrier_grenade)),
        ("CarrierSaveLoc", MonsterHandler::Callback(carrier_save_loc)),
        ("CarrierRail", MonsterHandler::Callback(carrier_rail)),
        ("carrier_attack_mg", MonsterHandler::Callback(carrier_attack_mg)),
        ("carrier_reattack_mg", MonsterHandler::Callback(carrier_reattack_mg)),
        ("carrier_attack_gren", MonsterHandler::Callback(carrier_attack_gren)),
        ("carrier_reattack_gren", MonsterHandler::Callback(carrier_reattack_gren)),
        ("carrier_prep_spawn", MonsterHandler::Callback(carrier_prep_spawn)),
        ("carrier_start_spawn", MonsterHandler::Callback(carrier_start_spawn)),
        ("carrier_ready_spawn", MonsterHandler::Callback(carrier_ready_spawn)),
        ("carrier_spawn_check", MonsterHandler::Callback(carrier_spawn_check)),
    ] {
        definition.callbacks.insert(name.to_string(), handler);
    }
    with_boss_explosion_callbacks(definition)
}
