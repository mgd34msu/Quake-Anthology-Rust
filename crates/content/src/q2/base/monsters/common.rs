//! Shared source monster callbacks (`src/content/q2/base/monsters/common.ts`).
//!
//! Shared source monster callbacks. id Software Quake II, GPL-2.0-or-later.

use qa_core::math::{Bounds, Vec3, add3, normalize3, scale3, sub3, vec3};

use crate::q2::foundation::host::{
    Q2EffectEvent, Q2MotionKind, Q2PresentationEvent, Q2SoundEvent, Q2SoundLoop,
};
use crate::q2::foundation::monsters::ai::{
    angles_vectors, enemy_body, enemy_eye, health, project_flash,
};
use crate::q2::foundation::monsters::gibs::{Q2GibOptions, throw_gib, throw_head};
use crate::q2::foundation::monsters::muzzle::muzzle_offset;
use crate::q2::foundation::monsters::types::{MonsterContext, MonsterHandler};
use crate::q2::support::contracts::{CombatTraitChanges, DeathReaction};

/// Humanoid bounds (`humanoidBounds`).
pub const HUMANOID_BOUNDS: Bounds = Bounds {
    min: Vec3 {
        x: -16.0,
        y: -16.0,
        z: -24.0,
    },
    max: Vec3 {
        x: 16.0,
        y: 16.0,
        z: 32.0,
    },
};

/// Default corpse bounds (`finishCorpse` fallback).
pub const CORPSE_BOUNDS: Bounds = Bounds {
    min: Vec3 {
        x: -16.0,
        y: -16.0,
        z: -24.0,
    },
    max: Vec3 {
        x: 16.0,
        y: 16.0,
        z: -8.0,
    },
};

/// Set a move (`move(name)`).
pub fn move_handler(name: &str) -> MonsterHandler {
    MonsterHandler::SetMove(name.to_string())
}

/// Play a sound (`sound(path, channel, attenuation)`).
pub fn sound_handler(path: &str, channel: i32, attenuation: f64) -> MonsterHandler {
    MonsterHandler::PlaySound {
        path: path.to_string(),
        channel,
        attenuation,
    }
}

/// Whether the enemy is alive (`aliveEnemy`).
pub fn alive_enemy(context: &mut MonsterContext) -> bool {
    if enemy_body(context).is_none() {
        return false;
    }
    let enemy = context.entity().enemy.clone();
    health(&mut *context.game, enemy.as_ref()) > 0.0
}

/// Switch to the damaged skin below half health (`damagedSkin`).
pub fn damaged_skin(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let max_health = context.entity().max_health;
    if health(&mut *context.game, Some(&actor)) < max_health / 2.0 {
        context.entity_mut().skin = 1;
    }
}

/// Emit a monster muzzle flash (`muzzle`).
pub fn monster_muzzle(context: &mut MonsterContext, flash: i32, direction: Vec3, origin: Vec3) {
    let actor = context.actor().clone();
    context.game.host_emit(Q2PresentationEvent::MonsterMuzzleflash {
        actor,
        flash,
        origin,
        direction,
    });
}

/// Aim a flash at the enemy eye with velocity lead (`shot`).
pub fn monster_shot(
    context: &mut MonsterContext,
    flash: usize,
    lead: f64,
) -> Option<(Vec3, Vec3)> {
    let enemy = enemy_body(context)?;
    let eye = enemy_eye(context)?;
    let edition = context.game.options.edition;
    let start = project_flash(context, muzzle_offset(edition, flash), None);
    let aim = add3(eye, scale3(enemy.velocity, lead as f32));
    Some((start, normalize3(sub3(aim, start))))
}

/// Aim a flash along the monster facing (`forwardShot`).
pub fn forward_shot(context: &mut MonsterContext, flash: usize) -> (Vec3, Vec3) {
    let edition = context.game.options.edition;
    let start = project_flash(context, muzzle_offset(edition, flash), None);
    let actor = context.actor().clone();
    let angles = context.game.body_of(actor).angles;
    (start, angles_vectors(angles).forward)
}

/// Gib below gib health (`standardGib`).
pub fn standard_gib(
    context: &mut MonsterContext,
    reaction: &DeathReaction,
    bones: i32,
    meats: i32,
    head: &str,
    attenuation: f64,
) -> bool {
    let actor = context.actor().clone();
    let gib_health = context.state().gib_health;
    if health(&mut *context.game, Some(&actor)) > gib_health {
        return false;
    }
    let damage = reaction.pain.damage;
    context.game.sound(&actor, "misc/udeath.wav", 2, 1.0, attenuation);
    for _ in 0..bones {
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/objects/gibs/bone/tris.md2",
            damage,
            Q2GibOptions::default(),
        );
    }
    for _ in 0..meats {
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/objects/gibs/sm_meat/tris.md2",
            damage,
            Q2GibOptions::default(),
        );
    }
    throw_head(actor.clone(), &mut *context.game, head, damage);
    context.state_mut().dead = true;
    context.state_mut().gibbed = true;
    true
}

/// Explode and remove the monster (`explode`).
pub fn monster_explode(context: &mut MonsterContext, path: &str) {
    let actor = context.actor().clone();
    context.game.sound(&actor, path, 2, 1.0, 1.0);
    let origin = context.game.body_of(actor.clone()).origin;
    context.game.host_emit(Q2PresentationEvent::Effect(Q2EffectEvent {
        effect: "q2:explosion1".to_string(),
        origin,
        direction: vec3(0.0, 0.0, 0.0),
        count: 1,
        color: 0,
    }));
    context.state_mut().dead = true;
    context.game.remove_actor(actor);
}

/// Start a monster loop sound (`loopSound`).
pub fn monster_loop_sound(context: &mut MonsterContext, path: &str) {
    let actor = context.actor().clone();
    let origin = context.game.body_of(actor.clone()).origin;
    context.game.host_emit(Q2PresentationEvent::Sound(Q2SoundEvent {
        actor: Some(actor),
        origin,
        path: path.to_string(),
        channel: 0,
        volume: 1.0,
        attenuation: 1.0,
        reliable: false,
        loop_: Q2SoundLoop::Start,
        loop_owner: None,
    }));
}

/// Begin a death animation (`beginDeath`).
pub fn begin_death(
    context: &mut MonsterContext,
    reaction: &DeathReaction,
    path: &str,
    animation: &str,
    bones: i32,
    meats: i32,
) {
    if standard_gib(
        context,
        reaction,
        bones,
        meats,
        "models/objects/gibs/head2/tris.md2",
        1.0,
    ) || context.state().dead
    {
        return;
    }
    let actor = context.actor().clone();
    context.game.sound(&actor, path, 2, 1.0, 1.0);
    context.state_mut().dead = true;
    context.state_mut().can_take_damage = true;
    let owned = context.game.owned_of(actor);
    context.game.host.combat().set_traits(
        &owned,
        &CombatTraitChanges {
            can_take_damage: Some(true),
            ..CombatTraitChanges::default()
        },
    );
    context.set_move(animation, true);
}

/// Settle a corpse (`finishCorpse`).
pub fn finish_corpse(context: &mut MonsterContext, bounds: Bounds) {
    let actor = context.actor().clone();
    context.state_mut().corpse = true;
    context.entity_mut().server_flags |= 2;
    let mut body = context.game.body_of(actor.clone());
    body.bounds = bounds;
    context.game.write_body(actor.clone(), &body, true);
    context.game.set_motion_kind(actor.clone(), Q2MotionKind::Toss);
    context.game.cancel_actor(actor);
}

/// Settle a corpse with default bounds (`finishCorpse(context)`).
pub fn finish_corpse_default(context: &mut MonsterContext) {
    finish_corpse(context, CORPSE_BOUNDS);
}
