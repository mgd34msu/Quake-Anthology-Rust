//! Black widow 2 (`src/content/q2/missionpacks/monsters/widow2.ts`).
//!
//! Original Rogue m_widow2.c. ZeniMax Media, GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::{add3, length3, normalize3, scale3, sub3, vec3, Bounds, Vec3};

use super::power_armor::{monster_power_armor, PowerArmorKind};
use super::spawn::rogue_spawn_callbacks;
use super::state::rogue_state;
use super::tables::rogue_widow2::{widow2_frame, widow2_moves};
use super::types::{mission_services, mission_weapons};
use super::widow_common::{
    widow_clear_powerups, widow_power_think, widow_powerups, widow_project, widow_restore_armor, widow_slots,
    widow_slots_left, widow_summon,
};
use super::widow_death::{
    widow_debris_callbacks, widow_explode_think, widow_explosion, widow_explosion_leg, widow_gib,
};
use crate::q2::base::monsters::common::{damaged_skin, move_handler};
use crate::q2::foundation::host::{Q2MonsterBeam, Q2MotionKind, Q2PresentationEvent, Q2Solid, Q2TraceRequest};
use crate::q2::foundation::monsters::ai::{
    angles_vectors, change_yaw, enemy_body, enemy_eye, health, in_front, project_flash, target_distance, vector_angles,
};
use crate::q2::foundation::monsters::gibs::{throw_gib, throw_head, Q2GibOptions};
use crate::q2::foundation::monsters::muzzle::muzzle_offset;
use crate::q2::foundation::monsters::types::{
    record_at, MonsterAttackState, MonsterContext, MonsterHandler, Q2MonsterDefinition,
};
use crate::q2::rerelease::monsters::common::{monster_flash, predicted_direction};
use crate::q2::support::contracts::{DeathReaction, PainReaction, TraceHit};

/// Tongue offsets (`tongueOffsets`).
const TONGUE_OFFSETS: [Vec3; 8] = [
    Vec3 {
        x: 17.48,
        y: 0.10,
        z: 68.92,
    },
    Vec3 {
        x: 17.47,
        y: 0.29,
        z: 68.91,
    },
    Vec3 {
        x: 17.45,
        y: 0.53,
        z: 68.87,
    },
    Vec3 {
        x: 17.42,
        y: 0.78,
        z: 68.81,
    },
    Vec3 {
        x: 17.39,
        y: 1.02,
        z: 68.75,
    },
    Vec3 {
        x: 17.37,
        y: 1.20,
        z: 68.70,
    },
    Vec3 {
        x: 17.36,
        y: 1.24,
        z: 68.71,
    },
    Vec3 {
        x: 17.37,
        y: 1.21,
        z: 68.72,
    },
];

/// Tongue okay (`tongueOkay`).
fn tongue_okay(start: Vec3, end: Vec3) -> bool {
    let delta = sub3(start, end);
    let mut pitch = vector_angles(delta).x;
    if pitch < -180.0 {
        pitch += 360.0;
    }
    length3(delta) <= 256.0 && pitch.abs() <= 30.0
}

/// Save beam (`saveBeam`).
fn widow2_save_beam(context: &mut MonsterContext) {
    let enemy = enemy_body(context);
    context.entity_mut().pos2 = match &enemy {
        None => vec3(0.0, 0.0, 0.0),
        Some(_) => context.entity().pos1,
    };
    context.entity_mut().pos1 = enemy.map(|enemy| enemy.origin).unwrap_or(vec3(0.0, 0.0, 0.0));
}

/// Run (`run`).
fn widow2_run(context: &mut MonsterContext) {
    context.state_mut().hold_frame = false;
    let stand_ground = context.state().stand_ground;
    context.set_move(
        if stand_ground {
            "widow2_move_stand"
        } else {
            "widow2_move_run"
        },
        false,
    );
}

/// Beam (`beam`).
fn widow2_beam(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let Some(enemy) = enemy_body(context) else {
        return;
    };
    let frame = context.entity().frame;
    if frame >= widow2_frame::SPAWN04 && frame <= widow2_frame::SPAWN14 {
        let index = frame - widow2_frame::SPAWN04;
        let flash = 200 + index;
        let edition = context.game.options.edition;
        let start = project_flash(context, muzzle_offset(edition, flash as usize), None);
        let angles = context.game.body_of(actor.clone()).angles;
        let pitch = vector_angles(sub3(enemy.origin, start)).x;
        let aimed = vec3(angles.x + pitch, angles.y - (-40.0 + index as f32 * 8.0), angles.z);
        let direction = angles_vectors(aimed).forward;
        let weapons = mission_weapons(&*context.game);
        weapons.fire_heat_beam(
            actor,
            &mut *context.game,
            start,
            direction,
            vec3(0.0, 0.0, 0.0),
            10.0,
            50.0,
        );
        monster_flash(context, flash, start, direction);
        return;
    }
    widow2_save_beam(context);
    let frame = context.entity().frame;
    let firing = frame >= widow2_frame::FIREB05 && frame <= widow2_frame::FIREB09;
    let flash = if firing {
        195 + frame - widow2_frame::FIREB05
    } else {
        195
    };
    let edition = context.game.options.edition;
    let start = project_flash(context, muzzle_offset(edition, flash as usize), None);
    let height = enemy_eye(context).map(|eye| eye.z).unwrap_or(enemy.origin.z) - enemy.origin.z;
    let pos2 = context.entity().pos2;
    let direction = normalize3(sub3(vec3(pos2.x, pos2.y, pos2.z + height - 10.0), start));
    let weapons = mission_weapons(&*context.game);
    weapons.fire_heat_beam(
        actor,
        &mut *context.game,
        start,
        direction,
        vec3(0.0, 0.0, 0.0),
        10.0,
        50.0,
    );
    if firing {
        monster_flash(context, flash, start, direction);
    }
}

/// Tongue pull (`tonguePull`).
fn widow2_tongue_pull(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let enemy = enemy_body(context);
    let enemy_id = context.entity().enemy.clone();
    let (Some(enemy), Some(enemy_id)) = (enemy, enemy_id) else {
        widow2_run(context);
        return;
    };
    let offset = *record_at(
        &TONGUE_OFFSETS,
        (context.entity().frame - widow2_frame::TONGS01) as usize,
    );
    let start = widow_project(&actor, &mut *context.game, offset);
    if !tongue_okay(start, enemy.origin) {
        return;
    }
    let Some(owned) = context.game.host.actors().resolve_owned(&enemy_id) else {
        return;
    };
    let origin = if enemy.ground.is_none() {
        enemy.origin
    } else {
        vec3(enemy.origin.x, enemy.origin.y, enemy.origin.z + 1.0)
    };
    let delta = sub3(context.game.body_of(actor.clone()).origin, origin);
    let velocity = if context.game.host.is_player(&enemy_id) {
        add3(enemy.velocity, scale3(normalize3(delta), 1000.0))
    } else {
        if context.game.monsters.states.contains_key(&enemy_id) {
            let mut target = MonsterContext::new(enemy_id.clone(), &mut *context.game);
            target.state_mut().ideal_yaw = f64::from(vector_angles(delta).y);
            change_yaw(&mut target);
        }
        scale3(angles_vectors(context.game.body_of(actor).angles).forward, 1000.0)
    };
    let mut moved = enemy;
    moved.origin = origin;
    moved.ground = None;
    moved.velocity = velocity;
    if let Some(angles) = context.game.host.bodies().read(owned.id()).map(|body| body.angles) {
        moved.angles = angles;
    }
    context.game.host.bodies().write(&owned, &moved);
    if context.game.entity(&enemy_id).is_some() {
        let motion = context.game.require_entity(&enemy_id).motion;
        context.game.set_motion_kind(enemy_id, motion);
    }
}

/// Check attack (`checkAttack`).
fn widow2_check_attack(context: &mut MonsterContext) -> bool {
    let actor = context.actor().clone();
    let enemy = enemy_body(context);
    let eye = enemy_eye(context);
    let enemy_id = context.entity().enemy.clone();
    let (Some(enemy), Some(eye), Some(enemy_id)) = (enemy, eye, enemy_id) else {
        return false;
    };
    widow_powerups(context);
    let distance = target_distance(context);
    if context.game.random() < 0.8 && widow_slots_left(context) >= 2 && distance > 150.0 {
        rogue_state(&mut *context.game, &actor).blocked = true;
        context.state_mut().attack_state = MonsterAttackState::Missile;
        return true;
    }
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
            if context.game.host.is_player(&enemy_id) && widow_slots_left(context) >= 2 {
                context.state_mut().attack_state = MonsterAttackState::Blind;
                return true;
            }
            let solid_none = context
                .game
                .entity(&enemy_id)
                .is_some_and(|enemy| enemy.solid == Q2Solid::None);
            if !solid_none || trace.fraction < 1.0 {
                return false;
            }
        }
    }
    let origin = context.game.body_of(actor.clone()).origin;
    context.state_mut().ideal_yaw = f64::from(vector_angles(sub3(enemy.origin, origin)).y);
    if context.entity().timestamp < context.game.host.now()
        && distance < 300.0
        && tongue_okay(
            widow_project(&actor, &mut *context.game, TONGUE_OFFSETS[0]),
            enemy.origin,
        )
    {
        if context.game.options.skill == 0 && (context.game.random() * 4.0).floor() as i32 != 0 {
            return false;
        }
        context.state_mut().attack_state = MonsterAttackState::Melee;
        return true;
    }
    if context.game.host.now() < context.state().attack_finished {
        return false;
    }
    let stand_ground = context.state().stand_ground;
    let chance = if stand_ground {
        0.4
    } else if distance < 1000.0 {
        0.8
    } else {
        0.5
    };
    if context.game.random() < chance
        || context
            .game
            .entity(&enemy_id)
            .is_some_and(|enemy| enemy.solid == Q2Solid::None)
    {
        context.state_mut().attack_state = MonsterAttackState::Missile;
        return true;
    }
    false
}

/// Initialize (`initialize`).
fn widow2_initialize(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let skill = f64::from(context.game.options.skill);
    let coop = if context.game.options.mode == crate::q2::foundation::host::Q2Mode::Coop {
        500.0 * skill
    } else {
        0.0
    };
    let max_health = 2800.0 + 1000.0 * skill + coop;
    context.entity_mut().max_health = max_health;
    let owned = context.game.owned_of(actor);
    context.game.host.combat().set_health(&owned, max_health);
    context.entity_mut().laser_immune = true;
    context.state_mut().ignore_shots = true;
    context.entity_mut().prethink = Some(widow_power_think);
    widow_slots(context);
    if context.game.options.skill == 3 {
        monster_power_armor(context, PowerArmorKind::Shield, 750.0);
    }
}

/// Search (`search`).
fn widow2_search(context: &mut MonsterContext) {
    if context.game.random() < 0.5 {
        let actor = context.actor().clone();
        context.game.sound(&actor, "bosshovr/bhvunqv1.wav", 2, 1.0, 0.0);
    }
}

/// Attack (`attack`).
fn widow2_attack(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let blocked = rogue_state(&mut *context.game, &actor).blocked;
    rogue_state(&mut *context.game, &actor).blocked = false;
    if enemy_body(context).is_none() {
        return;
    }
    let ready = context.game.host.now() >= context.state().attack_finished;
    if mission_services(&*context.game).bad_area(&actor) {
        let pre = context.game.random() < 0.75 || !ready;
        context.set_move(
            if pre {
                "widow2_move_attack_pre_beam"
            } else {
                "widow2_move_attack_disrupt"
            },
            false,
        );
        return;
    }
    widow_slots(context);
    if (context.state().attack_state == MonsterAttackState::Blind || blocked) && widow_slots_left(context) >= 2 {
        context.set_move("widow2_move_spawn", true);
        return;
    }
    let luck = context.game.random();
    let slots = widow_slots_left(context) >= 2;
    if target_distance(context) < 600.0 {
        if slots {
            if luck <= 0.4 {
                context.set_move("widow2_move_attack_pre_beam", true);
            } else {
                context.set_move(
                    if luck <= 0.7 && ready {
                        "widow2_move_attack_disrupt"
                    } else {
                        "widow2_move_spawn"
                    },
                    false,
                );
            }
            return;
        }
        context.set_move(
            if luck <= 0.5 || !ready {
                "widow2_move_attack_pre_beam"
            } else {
                "widow2_move_attack_disrupt"
            },
            false,
        );
        return;
    }
    if slots {
        if luck < 0.3 {
            context.set_move("widow2_move_attack_pre_beam", true);
        } else {
            context.set_move(
                if luck < 0.65 || !ready {
                    "widow2_move_spawn"
                } else {
                    "widow2_move_attack_disrupt"
                },
                false,
            );
        }
        return;
    }
    context.set_move(
        if luck < 0.45 || !ready {
            "widow2_move_attack_pre_beam"
        } else {
            "widow2_move_attack_disrupt"
        },
        false,
    );
}

/// Pain (`pain`).
fn widow2_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    damaged_skin(context);
    let skill = context.game.options.skill;
    if skill == 3 || context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 5.0;
    let actor = context.actor().clone();
    context.game.sound(
        &actor,
        if reaction.damage < 15.0 {
            "widow/bw2pain1.wav"
        } else if reaction.damage < 75.0 {
            "widow/bw2pain2.wav"
        } else {
            "widow/bw2pain3.wav"
        },
        2,
        1.0,
        0.0,
    );
    if reaction.damage >= 15.0
        && context.game.random()
            < if reaction.damage < 75.0 {
                0.6 - 0.2 * f64::from(skill)
            } else {
                0.75 - 0.1 * f64::from(skill)
            }
    {
        context.state_mut().manual_steering = false;
        context.set_move("widow2_move_pain", true);
    }
}

/// Die (`die`).
fn widow2_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    let actor = context.actor().clone();
    if health(&mut *context.game, Some(&actor)) <= context.state().gib_health {
        let clipped = reaction.pain.damage.min(100.0);
        context.game.sound(&actor, "misc/udeath.wav", 2, 1.0, 1.0);
        for _ in 0..2 {
            widow_gib(
                &actor,
                &mut *context.game,
                "models/objects/gibs/bone/tris.md2",
                clipped,
                true,
                None,
                false,
                "",
                false,
            );
        }
        for _ in 0..3 {
            widow_gib(
                &actor,
                &mut *context.game,
                "models/objects/gibs/sm_meat/tris.md2",
                clipped,
                true,
                None,
                false,
                "",
                false,
            );
        }
        for _ in 0..3 {
            widow_gib(
                &actor,
                &mut *context.game,
                "models/monsters/blackwidow2/gib1/tris.md2",
                clipped,
                false,
                None,
                true,
                "",
                false,
            );
            widow_gib(
                &actor,
                &mut *context.game,
                "models/monsters/blackwidow2/gib2/tris.md2",
                clipped,
                false,
                None,
                true,
                "misc/fhit3.wav",
                false,
            );
        }
        for _ in 0..2 {
            widow_gib(
                &actor,
                &mut *context.game,
                "models/monsters/blackwidow2/gib3/tris.md2",
                clipped,
                false,
                None,
                true,
                "",
                false,
            );
            widow_gib(
                &actor,
                &mut *context.game,
                "models/monsters/blackwidow/gib3/tris.md2",
                clipped,
                false,
                None,
                true,
                "",
                false,
            );
        }
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/objects/gibs/chest/tris.md2",
            clipped,
            Q2GibOptions::default(),
        );
        throw_head(actor, &mut *context.game, "models/objects/gibs/head2/tris.md2", clipped);
        context.state_mut().dead = true;
        context.state_mut().gibbed = true;
        return;
    }
    if context.state().dead {
        return;
    }
    context.game.sound(&actor, "widow/death.wav", 2, 1.0, 0.0);
    context.state_mut().dead = true;
    context.state_mut().can_take_damage = false;
    let owned = context.game.owned_of(actor.clone());
    context.game.set_combat_traits(
        &owned,
        &crate::q2::support::contracts::CombatTraitChanges {
            can_take_damage: Some(false),
            ..crate::q2::support::contracts::CombatTraitChanges::default()
        },
    );
    context.entity_mut().count = 0;
    let candidates: Vec<ActorId> = context
        .game
        .entities
        .values()
        .filter(|child| child.classname == "monster_stalker")
        .map(|child| child.actor.id().clone())
        .collect();
    let stalkers: Vec<ActorId> = candidates
        .into_iter()
        .filter(|stalker| health(&mut *context.game, Some(stalker)) > 0.0)
        .collect();
    for stalker in stalkers {
        let hp = health(&mut *context.game, Some(&stalker));
        let origin = enemy_body(context)
            .map(|enemy| enemy.origin)
            .unwrap_or_else(|| context.game.body_of(actor.clone()).origin);
        context.game.damage(
            stalker,
            actor.clone(),
            Some(actor.clone()),
            hp + 1.0,
            0.0,
            vec3(0.0, 0.0, 0.0),
            origin,
            vec3(0.0, 0.0, 0.0),
            0,
            8,
            None,
        );
    }
    widow_clear_powerups(context);
    context.set_move("widow2_move_death", true);
}

/// Beam target remove (`Widow2BeamTargetRemove`).
fn widow2_beam_target_remove(context: &mut MonsterContext) {
    context.entity_mut().pos1 = vec3(0.0, 0.0, 0.0);
    context.entity_mut().pos2 = vec3(0.0, 0.0, 0.0);
}

/// Reattack beam (`widow2_reattack_beam`).
fn widow2_reattack_beam(context: &mut MonsterContext) {
    context.state_mut().manual_steering = false;
    let enemy = context.entity().enemy.clone();
    let gate = match enemy {
        Some(enemy) => in_front(context, &enemy) && context.game.random() <= 0.5,
        None => false,
    };
    let moves = if gate {
        let beam = context.game.random() < 0.7 || widow_slots_left(context) < 2;
        if beam {
            "widow2_move_attack_beam"
        } else {
            "widow2_move_spawn"
        }
    } else {
        "widow2_move_attack_post_beam"
    };
    context.set_move(moves, true);
}

/// Save disrupt loc (`Widow2SaveDisruptLoc`).
fn widow2_save_disrupt_loc(context: &mut MonsterContext) {
    context.entity_mut().pos1 = enemy_eye(context).unwrap_or(vec3(0.0, 0.0, 0.0));
}

/// Disrupt (`WidowDisrupt`).
fn widow_disrupt(context: &mut MonsterContext) {
    let Some(enemy) = enemy_body(context) else {
        return;
    };
    let edition = context.game.options.edition;
    let start = project_flash(context, muzzle_offset(edition, 148), None);
    let pos1 = context.entity().pos1;
    let locked = length3(sub3(pos1, enemy.origin)) < 30.0;
    let direction = if locked {
        Some(normalize3(sub3(pos1, start)))
    } else {
        predicted_direction(context, start, 1200.0, true, 0.0)
    };
    let Some(direction) = direction else {
        return;
    };
    let enemy_id = if locked { context.entity().enemy.clone() } else { None };
    let weapons = mission_weapons(&*context.game);
    let actor = context.actor().clone();
    weapons.fire_tracker(
        actor,
        &mut *context.game,
        start,
        direction,
        20.0,
        if locked { 500.0 } else { 1200.0 },
        enemy_id,
    );
    monster_flash(context, 148, start, direction);
}

/// Disrupt reattack (`widow2_disrupt_reattack`).
fn widow2_disrupt_reattack(context: &mut MonsterContext) {
    if context.game.random() < 0.25 + 0.15 * f64::from(context.game.options.skill) {
        context.state_mut().next_frame = widow2_frame::FIREA01;
    }
}

/// Start spawn (`widow_start_spawn`).
fn widow_start_spawn(context: &mut MonsterContext) {
    context.state_mut().manual_steering = true;
}

/// Ready spawn (`widow2_ready_spawn`).
fn widow2_ready_spawn(context: &mut MonsterContext) {
    widow2_beam(context);
    widow_summon(context, true, true);
}

/// Spawn check (`widow2_spawn_check`).
fn widow2_spawn_check(context: &mut MonsterContext) {
    widow2_beam(context);
    widow_summon(context, true, false);
}

/// Tongue (`Widow2Tongue`).
fn widow2_tongue(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let enemy = enemy_body(context);
    let enemy_id = context.entity().enemy.clone();
    let (Some(enemy), Some(enemy_id)) = (enemy, enemy_id) else {
        return;
    };
    let offset = *record_at(
        &TONGUE_OFFSETS,
        (context.entity().frame - widow2_frame::TONGS01) as usize,
    );
    let start = widow_project(&actor, &mut *context.game, offset);
    if !tongue_okay(start, enemy.origin)
        && !tongue_okay(
            start,
            vec3(
                enemy.origin.x,
                enemy.origin.y,
                enemy.origin.z + enemy.bounds.max.z - 8.0,
            ),
        )
        && !tongue_okay(
            start,
            vec3(
                enemy.origin.x,
                enemy.origin.y,
                enemy.origin.z + enemy.bounds.min.z + 8.0,
            ),
        )
    {
        return;
    }
    let trace = context.game.host.trace(&Q2TraceRequest {
        start,
        end: enemy.origin,
        bounds: None,
        ignore: Some(actor.clone()),
        mask: 1 | 2 | 0x2000000 | 0x4000000,
        exclude: Vec::new(),
    });
    if !matches!(&trace.hit, TraceHit::Actor { actor } if *actor == enemy_id) {
        return;
    }
    context.game.sound(&actor, "brain/brnatck3.wav", 1, 1.0, 1.0);
    context.game.host_emit(Q2PresentationEvent::MonsterBeam {
        effect: Q2MonsterBeam::Parasite,
        actor: actor.clone(),
        start,
        end: enemy.origin,
    });
    context.game.damage(
        enemy_id,
        actor.clone(),
        Some(actor),
        2.0,
        0.0,
        sub3(start, enemy.origin),
        enemy.origin,
        vec3(0.0, 0.0, 0.0),
        0,
        8,
        None,
    );
}

/// Crunch (`Widow2Crunch`).
fn widow2_crunch(context: &mut MonsterContext) {
    if enemy_body(context).is_none() {
        widow2_run(context);
        return;
    }
    widow2_tongue_pull(context);
    let enemy = enemy_body(context);
    let fire_hit = context.weapons.fire_hit;
    let actor = context.actor().clone();
    let kick = if context.entity().frame != widow2_frame::TONGS07 {
        0.0
    } else if enemy.as_ref().is_some_and(|enemy| enemy.ground.is_none()) {
        250.0
    } else {
        500.0
    };
    let damage = 20.0 + (context.game.random() * 6.0).floor();
    fire_hit(actor, &mut *context.game, vec3(150.0, 0.0, 4.0), damage, kick);
}

/// Toss (`Widow2Toss`).
fn widow2_toss(context: &mut MonsterContext) {
    let timestamp = context.game.host.now() + 3.0;
    context.entity_mut().timestamp = timestamp;
}

/// Start searching (`widow2_start_searching`).
fn widow2_start_searching(context: &mut MonsterContext) {
    context.entity_mut().count = 0;
}

/// Keep searching (`widow2_keep_searching`).
fn widow2_keep_searching(context: &mut MonsterContext) {
    if context.entity().count <= 2 {
        context.set_move("widow2_move_dead", true);
        context.entity_mut().frame = widow2_frame::DTHSRH01;
        context.entity_mut().count += 1;
    } else {
        context.set_move("widow2_move_really_dead", true);
    }
}

/// Final death (`widow2_finaldeath`).
fn widow2_finaldeath(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.state_mut().corpse = true;
    context.state_mut().can_take_damage = true;
    let owned = context.game.owned_of(actor.clone());
    context.game.set_combat_traits(
        &owned,
        &crate::q2::support::contracts::CombatTraitChanges {
            can_take_damage: Some(true),
            ..crate::q2::support::contracts::CombatTraitChanges::default()
        },
    );
    let mut body = context.game.body_of(actor.clone());
    body.bounds = Bounds {
        min: vec3(-70.0, -70.0, 0.0),
        max: vec3(70.0, 70.0, 80.0),
    };
    context.game.write_body(actor.clone(), &body, true);
    context.game.set_motion_kind(actor.clone(), Q2MotionKind::Toss);
    context.game.cancel_actor(actor);
}

/// Create the widow2 definition (`createWidow2Definition`).
pub fn create_widow2_definition() -> Q2MonsterDefinition {
    let mut definition = Q2MonsterDefinition::new(
        "monster_widow2",
        "widow2",
        "models/monsters/blackwidow2/tris.md2",
        2800.0,
        -900.0,
        2500.0,
        Bounds {
            min: vec3(-70.0, -70.0, 0.0),
            max: vec3(70.0, 70.0, 144.0),
        },
        1.0,
        "widow2_move_stand",
        widow2_moves(),
        move_handler("widow2_move_stand"),
        move_handler("widow2_move_walk"),
        MonsterHandler::Callback(widow2_run),
        MonsterHandler::Callback(widow2_attack),
        widow2_die,
    );
    definition.yaw_speed = Some(30.0);
    definition.melee = Some(move_handler("widow2_move_tongs"));
    definition.check_attack = Some(widow2_check_attack);
    let mut source_callbacks = widow_debris_callbacks();
    for (key, think) in rogue_spawn_callbacks().think {
        source_callbacks.think.insert(key, think);
    }
    source_callbacks
        .think
        .insert("q2:rogue/widow2_powerups", widow_power_think);
    source_callbacks
        .think
        .insert("q2:rogue/WidowExplode", widow_explode_think);
    definition.source_callbacks = Some(source_callbacks);
    definition.initialize = Some(MonsterHandler::Callback(widow2_initialize));
    definition.restore = Some(MonsterHandler::Callback(widow_restore_armor));
    definition.search = Some(MonsterHandler::Callback(widow2_search));
    definition.pain = Some(widow2_pain);
    for (name, handler) in [
        ("widow2_run", MonsterHandler::Callback(widow2_run)),
        ("Widow2Beam", MonsterHandler::Callback(widow2_beam)),
        ("Widow2SaveBeamTarget", MonsterHandler::Callback(widow2_save_beam)),
        ("Widow2StartSweep", MonsterHandler::Callback(widow2_save_beam)),
        (
            "Widow2BeamTargetRemove",
            MonsterHandler::Callback(widow2_beam_target_remove),
        ),
        ("widow2_attack_beam", move_handler("widow2_move_attack_beam")),
        ("widow2_reattack_beam", MonsterHandler::Callback(widow2_reattack_beam)),
        (
            "Widow2SaveDisruptLoc",
            MonsterHandler::Callback(widow2_save_disrupt_loc),
        ),
        ("WidowDisrupt", MonsterHandler::Callback(widow_disrupt)),
        (
            "widow2_disrupt_reattack",
            MonsterHandler::Callback(widow2_disrupt_reattack),
        ),
        ("widow_start_spawn", MonsterHandler::Callback(widow_start_spawn)),
        ("widow2_ready_spawn", MonsterHandler::Callback(widow2_ready_spawn)),
        ("widow2_spawn_check", MonsterHandler::Callback(widow2_spawn_check)),
        ("Widow2Tongue", MonsterHandler::Callback(widow2_tongue)),
        ("Widow2TonguePull", MonsterHandler::Callback(widow2_tongue_pull)),
        ("Widow2Crunch", MonsterHandler::Callback(widow2_crunch)),
        ("Widow2Toss", MonsterHandler::Callback(widow2_toss)),
        (
            "WidowExplosion1",
            MonsterHandler::Callback(|context| widow_explosion(context, vec3(23.74, -37.67, 76.96))),
        ),
        (
            "WidowExplosion2",
            MonsterHandler::Callback(|context| widow_explosion(context, vec3(-20.49, 36.92, 73.52))),
        ),
        (
            "WidowExplosion3",
            MonsterHandler::Callback(|context| widow_explosion(context, vec3(2.11, 0.05, 92.20))),
        ),
        (
            "WidowExplosion4",
            MonsterHandler::Callback(|context| widow_explosion(context, vec3(-28.04, -35.57, -77.56))),
        ),
        (
            "WidowExplosion5",
            MonsterHandler::Callback(|context| widow_explosion(context, vec3(-20.11, -1.11, 40.76))),
        ),
        (
            "WidowExplosion6",
            MonsterHandler::Callback(|context| widow_explosion(context, vec3(-20.11, -1.11, 40.76))),
        ),
        (
            "WidowExplosion7",
            MonsterHandler::Callback(|context| widow_explosion(context, vec3(-20.11, -1.11, 40.76))),
        ),
        ("WidowExplosionLeg", MonsterHandler::Callback(widow_explosion_leg)),
        ("WidowExplode", MonsterHandler::Callback(widow2_explode_callback)),
        (
            "widow2_start_searching",
            MonsterHandler::Callback(widow2_start_searching),
        ),
        ("widow2_keep_searching", MonsterHandler::Callback(widow2_keep_searching)),
        ("widow2_finaldeath", MonsterHandler::Callback(widow2_finaldeath)),
    ] {
        definition.callbacks.insert(name.to_string(), handler);
    }
    definition
}

/// Widow explode callback (`WidowExplode`).
fn widow2_explode_callback(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    widow_explode_think(actor, &mut *context.game);
}
