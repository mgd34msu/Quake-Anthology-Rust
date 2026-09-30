//! Rerelease guardian (`src/content/q2/rerelease/monsters/guardian.ts`).
//!
//! ZeniMax Media, GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, normalize3, sub3, vec3};

use super::beam::fire_monster_beam;
use super::boss::{boss_explode, random_body_point};
use super::common::monster_flash;
use super::tables::flashes::rerelease_flash;
use super::tables::guardian::{guardian_frame, guardian_moves};
use crate::q2::base::monsters::common::{move_handler, sound_handler};
use crate::q2::foundation::host::{
    Q2EffectEvent, Q2GameServices, Q2PresentationEvent, Q2SoundEvent,
    Q2SoundLoop,
};
use crate::q2::foundation::monsters::ai::{
    enemy_eye, health, project_flash, target_distance, visible,
};
use crate::q2::foundation::monsters::gibs::{Q2GibOptions, throw_gib};
use crate::q2::foundation::monsters::muzzle::muzzle_offset;
use crate::q2::foundation::monsters::types::{
    MonsterContext, MonsterHandler, Q2MonsterDefinition,
};
use crate::q2::foundation::weapons::types::Mod;
use crate::q2::support::contracts::PainReaction;

/// Fire update (`guardianFireUpdate`).
fn guardian_fire_update(beam: ActorId, game: &mut Q2GameServices) {
    let Some(frame) = guardian_fire_frame(game, &beam) else {
        return;
    };
    game.require_entity_mut(&beam).movedir = fire_offset(frame);
    game.require_entity_mut(&beam).frame = 2 + (frame & 1);
}

/// Fire frame (`guardianFireFrame`).
fn guardian_fire_frame(game: &mut Q2GameServices, beam: &ActorId) -> Option<i32> {
    let owner = game.require_entity(beam).owner.clone();
    let owner = owner.as_ref().and_then(|owner| game.entity(owner))?;
    Some(owner.frame)
}

/// Fire offset (`fireOffset`).
fn fire_offset(frame: i32) -> qa_core::math::Vec3 {
    match frame % 4 {
        3 => vec3(-0.25, -0.25, 0.0),
        2 => vec3(0.25, -0.25, 0.0),
        1 => vec3(-0.25, 0.25, 0.0),
        _ => vec3(0.25, 0.25, 0.0),
    }
}

/// Spin sound (`spinSound`).
fn spin_sound(context: &mut MonsterContext, start: bool) {
    let actor = context.actor().clone();
    let origin = context.game.body_of(actor.clone()).origin;
    context.game.host_emit(Q2PresentationEvent::Sound(Q2SoundEvent {
        actor: Some(actor),
        origin,
        path: "bosshovr/bhvengn1.wav".to_string(),
        channel: 1,
        volume: 1.0,
        attenuation: 1.0,
        reliable: false,
        loop_: if start {
            Q2SoundLoop::Start
        } else {
            Q2SoundLoop::Stop
        },
        loop_owner: None,
    }));
}

/// Run (`run`).
fn guardian_run(context: &mut MonsterContext) {
    let stand_ground = context.state().stand_ground;
    context.set_move(
        if stand_ground {
            "guardian_move_stand"
        } else {
            "guardian_move_run"
        },
        false,
    );
}

/// Attack (`attack`).
fn guardian_attack(context: &mut MonsterContext) {
    if target_distance(context) < 90.0 {
        context.set_move("guardian_move_atk2_start", true);
    } else {
        context.set_move("guardian_move_atk1_start", true);
    }
}

/// Pain (`pain`).
fn guardian_pain(context: &mut MonsterContext, _reaction: &PainReaction) {
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    let actor = context.actor().clone();
    context.game.sound(&actor, "guardian/pain.wav", 2, 1.0, 1.0);
    let first = context.game.random() < 0.5;
    context.set_move(
        if first {
            "guardian_move_pain1"
        } else {
            "guardian_move_pain2"
        },
        false,
    );
}

/// Attack 1 (`guardian_atk1`).
fn guardian_atk1(context: &mut MonsterContext) {
    let timestamp = context.game.host.now() + 0.65 + context.game.random() * 1.5;
    context.entity_mut().timestamp = timestamp;
    context.set_move("guardian_move_atk1_spin", true);
}

/// Attack 1 charge (`guardian_atk1_charge`).
fn guardian_atk1_charge(context: &mut MonsterContext) {
    spin_sound(context, true);
    let actor = context.actor().clone();
    context.game.sound(&actor, "weapons/hyprbu1a.wav", 1, 1.0, 1.0);
}

/// Attack 1 finish (`guardian_atk1_finish`).
fn guardian_atk1_finish(context: &mut MonsterContext) {
    spin_sound(context, false);
    context.set_move("guardian_atk1_out", true);
}

/// Fire blaster (`guardian_fire_blaster`).
fn guardian_fire_blaster(context: &mut MonsterContext) {
    let Some(eye) = enemy_eye(context) else {
        return;
    };
    let actor = context.actor().clone();
    let flash = rerelease_flash::GUARDIAN_BLASTER;
    let start =
        project_flash(context, muzzle_offset(context.game.options.edition, flash as usize), None);
    let target = vec3(
        eye.x + (context.game.random() * 2.0 - 1.0) as f32 * 5.0,
        eye.y + (context.game.random() * 2.0 - 1.0) as f32 * 5.0,
        eye.z + (context.game.random() * 2.0 - 1.0) as f32 * 5.0,
    );
    let direction = normalize3(sub3(target, start));
    let fire_blaster = context.weapons.fire_blaster;
    let effects = if context.entity().frame % 4 == 0 { 64 } else { 0 };
    fire_blaster(
        actor,
        &mut *context.game,
        start,
        direction,
        2.0,
        1000.0,
        effects,
        false,
        Mod::BLASTER,
    );
    monster_flash(context, flash, start, direction);
    let enemy = context.entity().enemy.clone();
    if health(&mut *context.game, enemy.as_ref()) > 0.0
        && context.entity().frame == guardian_frame::ATK1_SPIN12
        && context.entity().timestamp > context.game.host.now()
        && visible(context, None)
    {
        context.state_mut().next_frame = guardian_frame::ATK1_SPIN5;
    }
}

/// Laser fire (`guardian_laser_fire`).
fn guardian_laser_fire(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "weapons/laser2.wav", 1, 1.0, 1.0);
    let secondary = context.entity().frame & 1 != 0;
    fire_monster_beam(context, 25.0, secondary, guardian_fire_update);
}

/// Kick (`guardian_kick`).
fn guardian_kick(context: &mut MonsterContext) {
    let fire_hit = context.weapons.fire_hit;
    let actor = context.actor().clone();
    if !fire_hit(actor, &mut *context.game, vec3(80.0, 0.0, -80.0), 85.0, 700.0) {
        let melee = context.game.host.now() + 1.0;
        context.state_mut().melee_time = melee;
    }
}

/// Dead (`guardian_dead`).
fn guardian_dead(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    for _ in 0..3 {
        let origin = random_body_point(&actor, &mut *context.game);
        context.game.host_emit(Q2PresentationEvent::Effect(Q2EffectEvent {
            effect: "q2:explosion1-big".to_string(),
            origin,
            direction: vec3(0.0, 0.0, 0.0),
            count: 1,
            color: 0,
        }));
    }
    for _ in 0..2 {
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/objects/gibs/sm_meat/tris.md2",
            125.0,
            Q2GibOptions::default(),
        );
    }
    for _ in 0..4 {
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/objects/gibs/sm_metal/tris.md2",
            125.0,
            Q2GibOptions {
                metallic: true,
                ..Q2GibOptions::default()
            },
        );
    }
    for i in 1..=6 {
        for _ in 0..2 {
            let model = format!("models/monsters/guardian/gib{i}.md2");
            throw_gib(
                actor.clone(),
                &mut *context.game,
                &model,
                125.0,
                Q2GibOptions {
                    metallic: true,
                    ..Q2GibOptions::default()
                },
            );
        }
    }
    throw_gib(
        actor,
        &mut *context.game,
        "models/monsters/guardian/gib7.md2",
        125.0,
        Q2GibOptions {
            metallic: true,
            head: true,
            ..Q2GibOptions::default()
        },
    );
    context.state_mut().gibbed = true;
}

/// Stand (`stand`).
fn guardian_stand(context: &mut MonsterContext) {
    context.set_move("guardian_move_stand", true);
}

/// Die (`die`).
fn guardian_die(context: &mut MonsterContext, _reaction: &crate::q2::support::contracts::DeathReaction) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "guardian/death.wav", 2, 1.0, 1.0);
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
    context.set_move("guardian_move_death", true);
}

/// Guardian definition (`guardianDefinition`).
pub fn guardian_definition() -> Q2MonsterDefinition {
    let mut definition = Q2MonsterDefinition::new(
        "monster_guardian",
        "guardian",
        "models/monsters/guardian/tris.md2",
        3500.0,
        -600.0,
        2000.0,
        Bounds {
            min: vec3(-40.0, -40.0, -44.0),
            max: vec3(40.0, 40.0, 84.0),
        },
        1.0,
        "guardian_move_stand",
        guardian_moves(),
        MonsterHandler::Callback(guardian_stand),
        move_handler("guardian_move_walk"),
        MonsterHandler::Callback(guardian_run),
        MonsterHandler::Callback(guardian_attack),
        guardian_die,
    );
    definition.yaw_speed = Some(20.0);
    definition.sight = Some(sound_handler("guardian/sight.wav", 2, 1.0));
    definition.search = Some(sound_handler("guardian/search.wav", 2, 1.0));
    definition.pain = Some(guardian_pain);
    for (name, handler) in [
        ("guardian_run", MonsterHandler::Callback(guardian_run)),
        (
            "guardian_footstep",
            sound_handler("zortemp/step.wav", 4, 1.0),
        ),
        ("BossExplode", MonsterHandler::Callback(boss_explode)),
        ("guardian_atk1", MonsterHandler::Callback(guardian_atk1)),
        ("guardian_atk1_charge", MonsterHandler::Callback(guardian_atk1_charge)),
        ("guardian_atk1_finish", MonsterHandler::Callback(guardian_atk1_finish)),
        ("guardian_atk2", move_handler("guardian_move_atk2_fire")),
        ("guardian_atk2_out", move_handler("guardian_move_atk2_out")),
        (
            "guardian_fire_blaster",
            MonsterHandler::Callback(guardian_fire_blaster),
        ),
        ("guardian_laser_fire", MonsterHandler::Callback(guardian_laser_fire)),
        ("guardian_kick", MonsterHandler::Callback(guardian_kick)),
        ("guardian_dead", MonsterHandler::Callback(guardian_dead)),
    ] {
        definition.callbacks.insert(name.to_string(), handler);
    }
    definition
}
