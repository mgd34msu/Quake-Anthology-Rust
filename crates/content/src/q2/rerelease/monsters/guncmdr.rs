//! Rerelease gun commander (`src/content/q2/rerelease/monsters/guncmdr.ts`).
//!
//! ZeniMax Media, GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::{add3, dot3, length3, normalize3, scale3, sub3, vec3};

use super::berserk::slam_radius_damage;
use super::common::{
    JumpNavigation, JumpResult, blocked_check_jump, blocked_check_platform,
    calculate_pitch_to_fire, check_gib, monster_flash, monster_jump_finished,
    predicted_direction, reacts_to_pain,
};
use super::tables::flashes::rerelease_flash;
use super::tables::guncmdr::{guncmdr_frame as frame, guncmdr_moves};
use crate::contract::{InventoryEntry, PoweredProtectionState};
use crate::q2::base::monsters::common::{move_handler, sound_handler};
use crate::q2::foundation::fields::number_field;
use crate::q2::foundation::host::{Q2Edition, Q2TraceRequest};
use crate::q2::foundation::monsters::ai::{
    angles_vectors, clear_shot, corpse, enemy_body, finish_dodge, health,
    project_flash, set_duck, target_distance, visible,
};
use crate::q2::foundation::monsters::gibs::{Q2GibOptions, throw_gib};
use crate::q2::foundation::monsters::muzzle::muzzle_offset;
use crate::q2::foundation::monsters::types::{
    MonsterAttackState, MonsterContext, MonsterHandler, Q2MonsterDefinition,
    bind_shared_power_cells,
};
use crate::q2::foundation::weapons::types::Q2GrenadeAdjustment;
use crate::q2::missionpacks::monsters::types::mission_weapons;
use crate::q2::support::contracts::{DeathReaction, PainReaction};

/// Run (`run`).
fn guncmdr_run(context: &mut MonsterContext) {
    finish_dodge(context);
    let stand_ground = context.state().stand_ground;
    context.set_move(
        if stand_ground {
            "guncmdr_move_stand"
        } else {
            "guncmdr_move_run"
        },
        false,
    );
}

/// Behind (`behind`).
fn guncmdr_behind(context: &mut MonsterContext, actor: Option<&ActorId>) -> bool {
    let other = actor.and_then(|actor| context.game.host.bodies().read(actor));
    let Some(other) = other else {
        return false;
    };
    let self_actor = context.actor().clone();
    let body = context.game.body_of(self_actor);
    let delta = sub3(other.origin, body.origin);
    dot3(
        normalize3(vec3(delta.x, delta.y, 0.0)),
        angles_vectors(body.angles).forward,
    ) < -0.4
}

/// Can advance (`canAdvance`).
fn guncmdr_can_advance(context: &mut MonsterContext) -> bool {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    let end = add3(body.origin, scale3(angles_vectors(body.angles).forward, 8.0));
    context.game.host.trace(&Q2TraceRequest {
        start: body.origin,
        end,
        bounds: Some(body.bounds),
        ignore: Some(actor),
        mask: 0x2020003,
        exclude: Vec::new(),
    }).fraction == 1.0
}

/// Fire chain (`fireChain`).
fn guncmdr_fire_chain_inner(context: &mut MonsterContext, immediate: bool) {
    context.set_move(
        if !context.state().stand_ground
            && enemy_body(context).is_some()
            && target_distance(context) > 400.0
            && guncmdr_can_advance(context)
        {
            "guncmdr_move_fire_chain_run"
        } else {
            "guncmdr_move_fire_chain"
        },
        immediate,
    );
}

/// Attack (`attack`).
fn guncmdr_attack(context: &mut MonsterContext) {
    finish_dodge(context);
    let Some(enemy) = enemy_body(context) else {
        return;
    };
    let distance = target_distance(context);
    let actor = context.actor().clone();
    let body = context.game.body_of(actor);
    if distance < 80.0 && context.state().melee_time < context.game.host.now() {
        context.set_move("guncmdr_move_attack_kick", false);
        return;
    }
    if (distance <= 100.0 || context.game.random() < 0.5)
        && clear_shot(context, muzzle_offset(Q2Edition::Rerelease, rerelease_flash::GUNCMDR_CHAINGUN_1 as usize))
    {
        context.set_move("guncmdr_move_attack_chain", false);
        return;
    }
    let aim = normalize3(sub3(enemy.origin, body.origin));
    let mortar_offset = muzzle_offset(Q2Edition::Rerelease, rerelease_flash::GUNCMDR_GRENADE_MORTAR_1 as usize);
    let front_offset = muzzle_offset(Q2Edition::Rerelease, rerelease_flash::GUNCMDR_GRENADE_FRONT_1 as usize);
    let mortar_start = project_flash(context, mortar_offset, None);
    if (distance >= 525.0
        || (body.origin.z + body.bounds.min.z - enemy.origin.z - enemy.bounds.max.z).abs() > 64.0)
        && clear_shot(context, mortar_offset)
        && calculate_pitch_to_fire(context, enemy.origin, mortar_start, aim, 850.0, 2.5, true, false).is_some()
    {
        context.set_move("guncmdr_move_attack_mortar", false);
        set_duck(context, true);
        return;
    }
    let front_start = project_flash(context, front_offset, None);
    if clear_shot(context, front_offset)
        && !context.state().stand_ground
        && calculate_pitch_to_fire(context, enemy.origin, front_start, aim, 600.0, 2.5, false, false).is_some()
    {
        context.set_move("guncmdr_move_attack_grenade_back", false);
        return;
    }
    if context.state().stand_ground {
        context.set_move("guncmdr_move_attack_chain", false);
    }
}

/// Jump (`jump`).
fn guncmdr_jump(context: &mut MonsterContext, high: bool) {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    let axes = angles_vectors(body.angles);
    let mut moved = body;
    moved.velocity = add3(
        moved.velocity,
        add3(
            scale3(axes.forward, if high { 150.0 } else { 100.0 }),
            scale3(axes.up, if high { 400.0 } else { 300.0 }),
        ),
    );
    context.game.write_body(actor, &moved, true);
}

/// Sidestep (`sidestep`).
fn guncmdr_sidestep(context: &mut MonsterContext) -> bool {
    let current = context.state().current_move.name.clone();
    let side = if context.state().lefty { "left" } else { "right" };
    if current == "guncmdr_move_fire_chain" || current == "guncmdr_move_fire_chain_run" {
        let moves = format!("guncmdr_move_fire_chain_dodge_{side}");
        context.set_move(&moves, false);
    } else if current == "guncmdr_move_attack_grenade_back" {
        context.entity_mut().count = context.entity().frame;
        let moves = format!("guncmdr_move_attack_grenade_back_dodge_{side}");
        context.set_move(&moves, false);
    } else if current == "guncmdr_move_attack_mortar" {
        context.entity_mut().count = context.entity().frame;
        context.set_move("guncmdr_move_attack_mortar_dodge", false);
    } else if current == "guncmdr_move_run" {
        context.set_move("guncmdr_move_run", false);
    } else {
        return false;
    }
    true
}

/// Bind armor (`bindArmor`).
fn guncmdr_bind_armor(context: &mut MonsterContext) {
    bind_shared_power_cells(context);
}

/// Initialize (`initialize`).
fn guncmdr_initialize(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.entity_mut().scale = 1.25;
    context.entity_mut().skin = 2;
    context.state_mut().scale = f64::from((f64::from(1.15f32) * 1.25) as f32);
    let mut body = context.game.body_of(actor.clone());
    body.bounds.min = vec3(-20.0, -20.0, -30.0);
    body.bounds.max = vec3(20.0, 20.0, 45.0);
    context.game.write_body(actor.clone(), &body, true);
    context.state_mut().normal_height = 45.0;
    context.entity_mut().view_height = 37;
    let owned = context.game.owned_of(actor.clone());
    context.game.set_combat_traits(
        &owned,
        &crate::q2::support::contracts::CombatTraitChanges {
            mass: Some(255.0 * 1.25),
            ..crate::q2::support::contracts::CombatTraitChanges::default()
        },
    );
    if !context.game.host.inventory().has(&actor) {
        let owned = context.game.owned_of(actor.clone());
        context.game.host.inventory().create(&owned, &[]);
    }
    let cells = number_field(&context.entity().spawn, "power_armor_power", 200.0);
    let armor_type = number_field(&context.entity().spawn, "power_armor_type", 2.0);
    let owned = context.game.owned_of(actor);
    context.game.host.inventory().configure(
        &owned,
        &InventoryEntry {
            item: "q2:monster-power".to_string(),
            count: cells,
            capacity: 200.0f64.max(cells),
            count_policy: None,
        },
    );
    guncmdr_bind_armor(context);
    let actor = context.actor().clone();
    let owned = context.game.owned_of(actor);
    let protection = if armor_type == 0.0 {
        PoweredProtectionState::None
    } else if armor_type == 1.0 {
        PoweredProtectionState::Screen { cells }
    } else {
        PoweredProtectionState::Shield { cells }
    };
    context.game.host.combat().set_powered_protection(&owned, &protection);
}

/// Duck (`duck`).
fn guncmdr_duck(context: &mut MonsterContext, _eta: f64) -> bool {
    let current = context.state().current_move.name.clone();
    if current == "guncmdr_move_jump" || current == "guncmdr_move_jump2" {
        return false;
    }
    if current.contains("_dodge") {
        set_duck(context, false);
        return false;
    }
    context.set_move("guncmdr_move_duck_attack", false);
    true
}

/// Blocked (`blocked`).
fn guncmdr_blocked(context: &mut MonsterContext, distance: f64) -> bool {
    if blocked_check_platform(context, distance) {
        return true;
    }
    let can_jump = context.entity().spawnflags & 8 == 0;
    let result = blocked_check_jump(context, distance, 192.0, 40.0, can_jump, JumpNavigation::None);
    if result == JumpResult::None {
        return false;
    }
    if result != JumpResult::Turn && enemy_body(context).is_some() {
        finish_dodge(context);
        context.set_move(
            if result == JumpResult::Up {
                "guncmdr_move_jump2"
            } else {
                "guncmdr_move_jump"
            },
            false,
        );
    }
    true
}

/// Pain (`pain`).
fn guncmdr_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    finish_dodge(context);
    let actor = context.actor().clone();
    let max_health = context.entity().max_health;
    if health(&mut *context.game, Some(&actor)) < max_health / 2.0 {
        context.entity_mut().skin |= 1;
    } else {
        context.entity_mut().skin &= !1;
    }
    let current = context.state().current_move.name.clone();
    if current == "guncmdr_move_jump"
        || current == "guncmdr_move_jump2"
        || current == "guncmdr_move_duck_attack"
    {
        return;
    }
    if context.game.host.now() < context.state().pain_time {
        guncmdr_pain_dodge(context, reaction);
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    let actor = context.actor().clone();
    let path = if context.game.random() < 0.5 {
        "guncmdr/gcdrpain2.wav"
    } else {
        "guncmdr/gcdrpain1.wav"
    };
    context.game.sound(&actor, path, 2, 1.0, 1.0);
    if !reacts_to_pain(context) {
        guncmdr_pain_dodge(context, reaction);
        return;
    }
    if reaction.damage < 35.0 {
        let choice = (context.game.random() * 4.0).floor() as i32;
        context.set_move(
            if choice == 0 {
                "guncmdr_move_pain3"
            } else if choice == 1 {
                "guncmdr_move_pain2"
            } else if choice == 2 {
                "guncmdr_move_pain1"
            } else {
                "guncmdr_move_pain7"
            },
            false,
        );
    } else {
        let attacker = reaction.attacker.clone();
        let back = guncmdr_behind(context, attacker.as_ref());
        let first = context.game.random() < 0.5;
        context.set_move(
            if back {
                "guncmdr_move_pain6"
            } else if first {
                "guncmdr_move_pain4"
            } else {
                "guncmdr_move_pain5"
            },
            false,
        );
        context.state_mut().pain_time += 1.5;
    }
    context.state_mut().manual_steering = false;
    if context.state().ducked {
        set_duck(context, false);
    }
}

/// Pain dodge (`dodge`).
fn guncmdr_pain_dodge(context: &mut MonsterContext, reaction: &PainReaction) {
    if context.game.random() < 0.3 {
        if let Some(attacker) = &reaction.attacker {
            let eta = context.game.host.frame_seconds();
            context.dodge(attacker.clone(), eta, None, false);
        }
    }
}

/// Die (`die`).
fn guncmdr_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    let actor = context.actor().clone();
    if check_gib(context) {
        context.game.sound(&actor, "misc/udeath.wav", 2, 1.0, 1.0);
        context.entity_mut().skin /= 2;
        let damage = reaction.pain.damage;
        for _ in 0..2 {
            throw_gib(
                actor.clone(),
                &mut *context.game,
                "models/objects/gibs/bone/tris.md2",
                damage,
                Q2GibOptions::default(),
            );
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
        for part in ["chest", "garm", "gun", "foot"] {
            let model = format!("models/monsters/gunner/gibs/{part}.md2");
            throw_gib(
                actor.clone(),
                &mut *context.game,
                &model,
                damage,
                Q2GibOptions {
                    skinned: true,
                    upright: part == "garm" || part == "gun",
                    ..Q2GibOptions::default()
                },
            );
        }
        let model = if context.state().current_move.name != "guncmdr_move_death5" {
            "models/objects/gibs/sm_meat/tris.md2"
        } else {
            "models/monsters/gunner/gibs/head.md2"
        };
        throw_gib(
            actor,
            &mut *context.game,
            model,
            damage,
            Q2GibOptions {
                head: true,
                skinned: true,
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
    context.game.sound(&actor, "guncmdr/gcdrdeath1.wav", 2, 1.0, 1.0);
    context.state_mut().dead = true;
    context.state_mut().can_take_damage = true;
    let owned = context.game.owned_of(actor.clone());
    context.game.set_combat_traits(
        &owned,
        &crate::q2::support::contracts::CombatTraitChanges {
            can_take_damage: Some(true),
            ..crate::q2::support::contracts::CombatTraitChanges::default()
        },
    );
    let current = context.state().current_move.name.clone();
    if current == "guncmdr_move_pain5" && context.entity().frame < frame::C_PAIN508
        || current == "guncmdr_move_pain6" && context.entity().frame < frame::C_PAIN607
    {
        return;
    }
    let body = context.game.body_of(actor.clone());
    let view_height = f64::from(context.entity().view_height);
    if (f64::from(body.origin.z) + view_height - f64::from(reaction.point.z)).abs() <= 4.0
        && body.velocity.z < 65.0
    {
        context.set_move("guncmdr_move_death5", false);
        let damage = reaction.pain.damage;
        let head = throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/monsters/gunner/gibs/head.md2",
            damage,
            Q2GibOptions::default(),
        );
        let inflictor = reaction
            .inflictor
            .clone()
            .and_then(|inflictor| context.game.host.bodies().read(&inflictor));
        if let Some(head) = head {
            let direction = normalize3(sub3(body.origin, inflictor.map(|inflictor| inflictor.origin).unwrap_or(body.origin)));
            let angular = context.game.require_entity(&head).angular_velocity;
            context.game.require_entity_mut(&head).angular_velocity = scale3(angular, 0.15);
            let mut moved = context.game.body_of(head.clone());
            moved.origin = add3(body.origin, vec3(0.0, 0.0, 24.0));
            moved.angles = body.angles;
            let flat = scale3(direction, 100.0);
            moved.velocity = vec3(flat.x, flat.y, 200.0);
            context.game.write_body(head, &moved, true);
        }
    } else if guncmdr_behind(context, reaction.inflictor.as_ref()) {
        let current = context.state().current_move.name.clone();
        let choice = (context.game.random() * if current == "guncmdr_move_pain6" { 2.0 } else { 3.0 }).floor() as i32;
        context.set_move(
            if choice == 0 {
                "guncmdr_move_death3"
            } else if choice == 1 {
                "guncmdr_move_death7"
            } else {
                "guncmdr_move_pain6"
            },
            false,
        );
    } else {
        let current = context.state().current_move.name.clone();
        let first = (context.game.random() * if current == "guncmdr_move_pain5" { 1.0 } else { 2.0 }).floor() as i32 == 0;
        context.set_move(
            if first {
                "guncmdr_move_death4"
            } else {
                "guncmdr_move_pain5"
            },
            false,
        );
    }
}

/// Fidget (`guncmdr_fidget`).
fn guncmdr_fidget(context: &mut MonsterContext) {
    if !context.state().stand_ground
        && context.entity().enemy.is_none()
        && context.game.random() <= 0.05
    {
        context.set_move("guncmdr_move_fidget", false);
    }
}

/// Pain5 to death1 (`guncmdr_pain5_to_death1`).
fn guncmdr_pain5_to_death1(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    if health(&mut *context.game, Some(&actor)) < 0.0 {
        context.set_move("guncmdr_move_death1", false);
    }
}

/// Pain5 to death2 (`guncmdr_pain5_to_death2`).
fn guncmdr_pain5_to_death2(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    if health(&mut *context.game, Some(&actor)) < 0.0 && context.game.random() < 0.5 {
        context.set_move("guncmdr_move_death2", false);
    }
}

/// Pain6 to death6 (`guncmdr_pain6_to_death6`).
fn guncmdr_pain6_to_death6(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    if health(&mut *context.game, Some(&actor)) < 0.0 {
        context.set_move("guncmdr_move_death6", false);
    }
}

/// Dead (`guncmdr_dead`).
fn guncmdr_dead(context: &mut MonsterContext) {
    corpse(context);
    let scale = context.entity().scale as f32;
    let actor = context.actor().clone();
    let mut body = context.game.body_of(actor.clone());
    body.bounds.min = scale3(vec3(-16.0, -16.0, -24.0), scale);
    body.bounds.max = scale3(vec3(16.0, 16.0, -8.0), scale);
    context.game.write_body(actor, &body, true);
}

/// Shrink (`guncmdr_shrink`).
fn guncmdr_shrink(context: &mut MonsterContext) {
    context.entity_mut().server_flags |= 2;
    let scale = context.entity().scale as f32;
    let actor = context.actor().clone();
    let mut body = context.game.body_of(actor.clone());
    body.bounds.max.z = -4.0 * scale;
    context.game.write_body(actor, &body, true);
}

/// Fire chain (`guncmdr_fire_chain`).
fn guncmdr_fire_chain(context: &mut MonsterContext) {
    guncmdr_fire_chain_inner(context, true);
}

/// Refire chain (`guncmdr_refire_chain`).
fn guncmdr_refire_chain(context: &mut MonsterContext) {
    finish_dodge(context);
    context.state_mut().attack_state = MonsterAttackState::Straight;
    let enemy = context.entity().enemy.clone();
    if health(&mut *context.game, enemy.as_ref()) > 0.0
        && visible(context, None)
        && context.game.random() <= 0.5
    {
        guncmdr_fire_chain_inner(context, false);
    } else {
        context.set_move("guncmdr_move_endfire_chain", false);
    }
}

/// Chaingun fire (`GunnerCmdrFire`).
fn gunner_cmdr_fire(context: &mut MonsterContext) {
    if enemy_body(context).is_none() {
        return;
    }
    let actor = context.actor().clone();
    let frame_now = context.entity().frame;
    let id = if frame_now >= frame::C_ATTACK401 && frame_now <= frame::C_ATTACK505 {
        rerelease_flash::GUNCMDR_CHAINGUN_2
    } else {
        rerelease_flash::GUNCMDR_CHAINGUN_1
    };
    let start = project_flash(context, muzzle_offset(Q2Edition::Rerelease, id as usize), None);
    let offset = context.game.random() * 0.3;
    let Some(aim) = predicted_direction(context, start, 800.0, false, offset) else {
        return;
    };
    let direction = vec3(
        aim.x + (context.game.random() * 2.0 - 1.0) as f32 * 0.025,
        aim.y + (context.game.random() * 2.0 - 1.0) as f32 * 0.025,
        aim.z + (context.game.random() * 2.0 - 1.0) as f32 * 0.025,
    );
    let weapons = mission_weapons(&*context.game);
    weapons.fire_flechette(actor, &mut *context.game, start, direction, 4.0, 800.0, 2.0);
    monster_flash(context, id, start, direction);
}

/// Grenade shot (`GunnerCmdrGrenade` shot record).
struct GrenadeShot {
    /// Frame.
    frame: i32,
    /// Spread.
    spread: f64,
    /// Flash.
    id: i32,
    /// Kind.
    kind: &'static str,
}

/// Chaingun grenade (`GunnerCmdrGrenade`).
fn gunner_cmdr_grenade(context: &mut MonsterContext) {
    let Some(enemy) = enemy_body(context) else {
        return;
    };
    let shots = [
        GrenadeShot { frame: frame::C_ATTACK205, spread: -0.1, id: rerelease_flash::GUNCMDR_GRENADE_MORTAR_1, kind: "mortar" },
        GrenadeShot { frame: frame::C_ATTACK208, spread: 0.0, id: rerelease_flash::GUNCMDR_GRENADE_MORTAR_2, kind: "mortar" },
        GrenadeShot { frame: frame::C_ATTACK211, spread: 0.1, id: rerelease_flash::GUNCMDR_GRENADE_MORTAR_3, kind: "mortar" },
        GrenadeShot { frame: frame::C_ATTACK304, spread: -0.1, id: rerelease_flash::GUNCMDR_GRENADE_FRONT_1, kind: "front" },
        GrenadeShot { frame: frame::C_ATTACK307, spread: 0.0, id: rerelease_flash::GUNCMDR_GRENADE_FRONT_2, kind: "front" },
        GrenadeShot { frame: frame::C_ATTACK310, spread: 0.1, id: rerelease_flash::GUNCMDR_GRENADE_FRONT_3, kind: "front" },
        GrenadeShot { frame: frame::C_ATTACK911, spread: 0.25, id: rerelease_flash::GUNCMDR_GRENADE_CROUCH_1, kind: "crouch" },
        GrenadeShot { frame: frame::C_ATTACK912, spread: 0.0, id: rerelease_flash::GUNCMDR_GRENADE_CROUCH_2, kind: "crouch" },
        GrenadeShot { frame: frame::C_ATTACK913, spread: -0.25, id: rerelease_flash::GUNCMDR_GRENADE_CROUCH_3, kind: "crouch" },
    ];
    let frame_now = context.entity().frame;
    let Some(shot) = shots.iter().find(|shot| shot.frame == frame_now) else {
        return;
    };
    let manual = context.state().manual_steering;
    if manual && !visible(context, None) && length3(context.state().blind_fire_target) == 0.0 {
        return;
    }
    let target = if manual && !visible(context, None) {
        context.state().blind_fire_target
    } else {
        enemy.origin
    };
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    let axes = angles_vectors(body.angles);
    let start = project_flash(context, muzzle_offset(Q2Edition::Rerelease, shot.id as usize), None);
    let mut delta = sub3(target, body.origin);
    let mut pitch = 0.0;
    if shot.kind != "crouch" {
        let distance = length3(delta);
        if distance > 512.0 && delta.z < 64.0 && delta.z > -64.0 {
            delta.z += distance - 512.0;
        }
        pitch = normalize3(delta).z.clamp(-0.5, 0.4) as f64;
        if enemy.origin.z + enemy.bounds.min.z - body.origin.z - body.bounds.max.z > 16.0
            && shot.kind == "mortar"
        {
            pitch += 0.5;
        }
    }
    if shot.kind == "front" {
        pitch -= 0.05;
    }
    let direction = if shot.kind == "crouch" {
        predicted_direction(context, start, 800.0, false, 0.0)
    } else {
        Some(add3(axes.forward, scale3(axes.up, pitch as f32)))
    };
    let Some(direction) = direction else {
        return;
    };
    let aim = normalize3(add3(direction, scale3(axes.right, shot.spread as f32)));
    if shot.kind == "crouch" {
        let weapons = mission_weapons(&*context.game);
        for i in 0..3 {
            let actor = context.actor().clone();
            weapons.fire_ion_ripper(
                actor,
                &mut *context.game,
                start,
                add3(aim, scale3(axes.right, -0.25 + 0.125 * f64::from(i + 1) as f32)),
                15.0,
                800.0,
                0x100000,
            );
        }
    } else {
        let speed = if shot.kind == "mortar" { 850.0 } else { 600.0 };
        let predicted = calculate_pitch_to_fire(
            context,
            target,
            start,
            aim,
            speed,
            2.5,
            shot.kind == "mortar",
            false,
        );
        let right = (context.game.random() * 2.0 - 1.0) * 10.0;
        let up = if predicted.is_none() {
            200.0 + (context.game.random() * 2.0 - 1.0) * 10.0
        } else {
            context.game.random() * 10.0
        };
        let gravity = context.game.host.gravity();
        let fire_grenade = context.weapons.fire_grenade;
        let actor = context.actor().clone();
        fire_grenade(
            actor,
            &mut *context.game,
            start,
            predicted.unwrap_or(aim),
            50.0,
            speed,
            2.5,
            90.0,
            false,
            false,
            true,
            Some(Q2GrenadeAdjustment { right, up, gravity }),
        );
    }
    monster_flash(context, shot.id, start, aim);
}

/// Mortar dodge resume (`guncmdr_grenade_mortar_resume`).
fn guncmdr_grenade_mortar_resume(context: &mut MonsterContext) {
    context.set_move("guncmdr_move_attack_mortar", false);
    context.state_mut().attack_state = MonsterAttackState::Straight;
    context.entity_mut().frame = context.entity().count;
}

/// Back dodge resume (`guncmdr_grenade_back_dodge_resume`).
fn guncmdr_grenade_back_dodge_resume(context: &mut MonsterContext) {
    context.set_move("guncmdr_move_attack_grenade_back", false);
    context.state_mut().attack_state = MonsterAttackState::Straight;
    context.entity_mut().frame = context.entity().count;
}

/// Kick finished (`guncmdr_kick_finished`).
fn guncmdr_kick_finished(context: &mut MonsterContext) {
    let melee = context.game.host.now() + 3.0;
    context.state_mut().melee_time = melee;
    guncmdr_attack(context);
}

/// Kick (`guncmdr_kick`).
fn guncmdr_kick(context: &mut MonsterContext) {
    let fire_hit = context.weapons.fire_hit;
    let actor = context.actor().clone();
    if fire_hit(actor.clone(), &mut *context.game, vec3(80.0, 0.0, -32.0), 15.0, 400.0) {
        let enemy = context.entity().enemy.clone();
        if enemy.as_ref().is_some_and(|enemy| context.game.host.is_player(enemy)) {
            let enemy = enemy.expect("guncmdr kick enemy");
            let body = context.game.host.bodies().read(&enemy);
            let owned = context.game.host.actors().resolve_owned(&enemy);
            if let (Some(body), Some(owned)) = (body, owned) {
                if body.velocity.z < 270.0 {
                    let mut moved = body;
                    moved.velocity.z = 270.0;
                    context.game.host.bodies().write(&owned, &moved);
                }
            }
        }
    }
}

/// Jump now (`guncmdr_jump_now`).
fn guncmdr_jump_now(context: &mut MonsterContext) {
    guncmdr_jump(context, false);
}

/// Second jump now (`guncmdr_jump2_now`).
fn guncmdr_jump2_now(context: &mut MonsterContext) {
    guncmdr_jump(context, true);
}

/// Jump wait land (`guncmdr_jump_wait_land`).
fn guncmdr_jump_wait_land(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let landed =
        context.game.body_of(actor).ground.is_some() || monster_jump_finished(context);
    context.state_mut().next_frame = context.entity().frame + if landed { 1 } else { 0 };
}

/// Counter (`GunnerCmdrCounter`).
fn gunner_cmdr_counter(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    let direction = angles_vectors(body.angles).forward;
    let end = project_flash(context, vec3(20.0, 0.0, 14.0), None);
    let trace = context.game.host.trace(&Q2TraceRequest {
        start: body.origin,
        end,
        bounds: None,
        ignore: Some(actor),
        mask: 3,
        exclude: Vec::new(),
    });
    context.game.host_emit(crate::q2::foundation::host::Q2PresentationEvent::Effect(
        crate::q2::foundation::host::Q2EffectEvent {
            effect: "q2:berserk-slam".to_string(),
            origin: trace.end,
            direction,
            count: 1,
            color: 0,
        },
    ));
    slam_radius_damage(context, trace.end, 15.0, 250.0, 200.0);
}

/// Create the gun commander definition (`createGunCommanderDefinition`).
pub fn create_gun_commander_definition() -> Q2MonsterDefinition {
    let mut definition = Q2MonsterDefinition::new(
        "monster_guncmdr",
        "guncmdr",
        "models/monsters/gunner/tris.md2",
        325.0,
        -175.0,
        255.0,
        crate::q2::base::monsters::common::HUMANOID_BOUNDS,
        f64::from(1.15f32),
        "guncmdr_move_stand",
        guncmdr_moves(),
        move_handler("guncmdr_move_stand"),
        move_handler("guncmdr_move_walk"),
        MonsterHandler::Callback(guncmdr_run),
        MonsterHandler::Callback(guncmdr_attack),
        guncmdr_die,
    );
    definition.bounds.min = vec3(-16.0, -16.0, -24.0);
    definition.bounds.max = vec3(16.0, 16.0, 36.0);
    definition.sidestep = Some(guncmdr_sidestep);
    definition.sight = Some(sound_handler("guncmdr/sight1.wav", 2, 1.0));
    definition.search = Some(sound_handler("guncmdr/gcdrsrch1.wav", 2, 1.0));
    definition.initialize = Some(MonsterHandler::Callback(guncmdr_initialize));
    definition.restore = Some(MonsterHandler::Callback(guncmdr_bind_armor));
    definition.duck = Some(guncmdr_duck);
    definition.blocked = Some(guncmdr_blocked);
    definition.pain = Some(guncmdr_pain);
    for (name, handler) in [
        ("guncmdr_stand", move_handler("guncmdr_move_stand")),
        ("guncmdr_run", MonsterHandler::Callback(guncmdr_run)),
        ("guncmdr_idlesound", sound_handler("guncmdr/gcdridle1.wav", 2, 2.0)),
        ("guncmdr_opengun", sound_handler("guncmdr/gcdratck1.wav", 2, 2.0)),
        ("guncmdr_fidget", MonsterHandler::Callback(guncmdr_fidget)),
        ("guncmdr_pain5_to_death1", MonsterHandler::Callback(guncmdr_pain5_to_death1)),
        ("guncmdr_pain5_to_death2", MonsterHandler::Callback(guncmdr_pain5_to_death2)),
        ("guncmdr_pain6_to_death6", MonsterHandler::Callback(guncmdr_pain6_to_death6)),
        ("guncmdr_dead", MonsterHandler::Callback(guncmdr_dead)),
        ("guncmdr_shrink", MonsterHandler::Callback(guncmdr_shrink)),
        ("guncmdr_fire_chain", MonsterHandler::Callback(guncmdr_fire_chain)),
        ("guncmdr_refire_chain", MonsterHandler::Callback(guncmdr_refire_chain)),
        ("GunnerCmdrFire", MonsterHandler::Callback(gunner_cmdr_fire)),
        ("GunnerCmdrGrenade", MonsterHandler::Callback(gunner_cmdr_grenade)),
        ("guncmdr_grenade_mortar_resume", MonsterHandler::Callback(guncmdr_grenade_mortar_resume)),
        ("guncmdr_grenade_back_dodge_resume", MonsterHandler::Callback(guncmdr_grenade_back_dodge_resume)),
        ("guncmdr_kick_finished", MonsterHandler::Callback(guncmdr_kick_finished)),
        ("guncmdr_kick", MonsterHandler::Callback(guncmdr_kick)),
        ("guncmdr_jump_now", MonsterHandler::Callback(guncmdr_jump_now)),
        ("guncmdr_jump2_now", MonsterHandler::Callback(guncmdr_jump2_now)),
        ("guncmdr_jump_wait_land", MonsterHandler::Callback(guncmdr_jump_wait_land)),
        ("GunnerCmdrCounter", MonsterHandler::Callback(gunner_cmdr_counter)),
    ] {
        definition.callbacks.insert(name.to_string(), handler);
    }
    definition
}
