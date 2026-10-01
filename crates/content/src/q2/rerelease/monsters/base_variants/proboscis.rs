//! Rerelease parasite proboscis (`src/content/q2/rerelease/monsters/base-variants/proboscis.ts`).
//!
//! ZeniMax Media, GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::{add3, dot3, length3, normalize3, scale3, sub3, vec3, Vec3};

use super::super::common::{predicted_direction, rerelease_random};
use super::super::tables::parasite::parasite_frame;
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::host::{
    Q2GameServices, Q2MotionKind, Q2PresentationEvent, Q2Solid, Q2SoundEvent, Q2SoundLoop, Q2TraceRequest,
};
use crate::q2::foundation::monsters::ai::{health, project_flash, vector_angles};
use crate::q2::foundation::monsters::types::MonsterContext;
use crate::q2::support::contracts::{AttackCause, DeathReaction, TouchContact, TraceContact, TraceHit};

/// Break offsets (`breakOffsets`).
const BREAK_OFFSETS: [Vec3; 20] = [
    Vec3 { x: 7.0, y: 0.0, z: 7.0 },
    Vec3 {
        x: 6.3,
        y: 14.5,
        z: 4.0,
    },
    Vec3 { x: 8.5, y: 0.0, z: 5.6 },
    Vec3 {
        x: 5.0,
        y: -15.25,
        z: 4.0,
    },
    Vec3 {
        x: 9.5,
        y: -1.8,
        z: 5.9,
    },
    Vec3 {
        x: 6.2,
        y: 14.0,
        z: 4.0,
    },
    Vec3 {
        x: 12.25,
        y: 7.5,
        z: 1.4,
    },
    Vec3 {
        x: 13.8,
        y: 0.0,
        z: -2.4,
    },
    Vec3 {
        x: 13.8,
        y: 0.0,
        z: -4.0,
    },
    Vec3 {
        x: 0.1,
        y: 0.0,
        z: -0.7,
    },
    Vec3 { x: 5.0, y: 0.0, z: 3.7 },
    Vec3 {
        x: 11.0,
        y: 0.0,
        z: 4.0,
    },
    Vec3 {
        x: 13.5,
        y: 0.0,
        z: -4.0,
    },
    Vec3 {
        x: 13.5,
        y: 0.0,
        z: -4.0,
    },
    Vec3 {
        x: 0.2,
        y: 0.0,
        z: -0.7,
    },
    Vec3 { x: 3.9, y: 0.0, z: 3.6 },
    Vec3 { x: 8.5, y: 0.0, z: 5.0 },
    Vec3 {
        x: 14.0,
        y: 0.0,
        z: -4.0,
    },
    Vec3 {
        x: 14.0,
        y: 0.0,
        z: -4.0,
    },
    Vec3 {
        x: 0.1,
        y: 0.0,
        z: -0.5,
    },
];

/// Drain offsets (`drainOffsets`).
const DRAIN_OFFSETS: [Vec3; 13] = [
    Vec3 {
        x: -1.7,
        y: 0.0,
        z: 1.2,
    },
    Vec3 {
        x: -2.2,
        y: 0.0,
        z: -0.6,
    },
    Vec3 { x: 7.7, y: 0.0, z: 7.2 },
    Vec3 { x: 7.2, y: 0.0, z: 5.7 },
    Vec3 { x: 6.2, y: 0.0, z: 7.8 },
    Vec3 { x: 4.7, y: 0.0, z: 6.7 },
    Vec3 { x: 5.0, y: 0.0, z: 9.0 },
    Vec3 { x: 5.0, y: 0.0, z: 7.0 },
    Vec3 {
        x: 5.0,
        y: 0.0,
        z: 10.5,
    },
    Vec3 { x: 4.5, y: 0.0, z: 9.7 },
    Vec3 {
        x: 1.5,
        y: 0.0,
        z: 12.0,
    },
    Vec3 {
        x: 2.9,
        y: 0.0,
        z: 11.0,
    },
    Vec3 { x: 2.1, y: 0.0, z: 7.6 },
];

/// Proboscis start (`start`).
fn proboscis_start(context: &mut MonsterContext) -> Vec3 {
    let frame = context.entity().frame;
    let offset = usize::try_from(frame - parasite_frame::BREAK01)
        .ok()
        .and_then(|index| BREAK_OFFSETS.get(index))
        .or_else(|| {
            usize::try_from(frame - parasite_frame::DRAIN01)
                .ok()
                .and_then(|index| DRAIN_OFFSETS.get(index))
        })
        .copied()
        .unwrap_or(vec3(8.0, 0.0, 6.0));
    project_flash(context, offset, None)
}

/// Proboscis start for an owner actor.
fn proboscis_start_for(game: &mut Q2GameServices, owner: &ActorId) -> Vec3 {
    let mut context = MonsterContext::new(owner.clone(), game);
    proboscis_start(&mut context)
}

/// Reset (`reset`).
pub fn proboscis_reset(tip: ActorId, game: &mut Q2GameServices) {
    let entity = game.require_entity(&tip);
    let (owner, segment) = (entity.owner.clone(), entity.proboscus.clone());
    if let Some(owner) = owner {
        if game.entity(&owner).is_some() {
            game.require_entity_mut(&owner).proboscus = None;
        }
    }
    if let Some(segment) = segment {
        if game.entity(&segment).is_some() {
            game.remove_actor(segment);
        }
    }
    game.remove_actor(tip);
}

/// Die (`die`).
fn proboscis_die(tip: ActorId, game: &mut Q2GameServices, _reaction: DeathReaction) {
    let cause = game.require_entity(&tip).last_attack.clone().map(|attack| attack.cause);
    if matches!(cause, Some(AttackCause::Q2 { means_of_death: 20, .. })) {
        proboscis_reset(tip, game);
    }
}

/// Retract (`retract`).
pub fn proboscis_retract(tip: ActorId, game: &mut Q2GameServices) {
    let owner = game.require_entity(&tip).owner.clone();
    if let Some(owner) = owner {
        if let Some(state) = game.monsters.states.get_mut(&owner) {
            if state.current_move.name == "parasite_move_fire_proboscis" {
                state.next_frame = parasite_frame::DRAIN12;
            }
        }
    }
    let entity = game.require_entity_mut(&tip);
    if entity.style != 2 {
        entity.speed *= 2.0;
    }
    entity.style = 2;
    game.set_motion_kind(tip.clone(), Q2MotionKind::Stationary);
    game.set_solid(tip.clone(), Q2Solid::None);
    game.link_actor(tip);
}

/// Hit (`hit`).
#[allow(clippy::too_many_arguments)]
fn proboscis_hit(
    tip: ActorId,
    game: &mut Q2GameServices,
    other: ActorId,
    point: Vec3,
    normal: Vec3,
    start_solid: bool,
) {
    let owner = game.require_entity(&tip).owner.clone();
    let Some(owner) = owner.filter(|owner| game.monsters.states.contains_key(owner)) else {
        return;
    };
    if game
        .monsters
        .states
        .get(&owner)
        .expect("proboscis owner")
        .current_move
        .name
        != "parasite_move_fire_proboscis"
    {
        return;
    }
    let body = game.body_of(tip.clone());
    let target = game.host.bodies().read(&other);
    let owner_enemy = game.require_entity(&owner).enemy.clone();
    let position = if target.is_some() && (game.host.is_player(&other) || owner_enemy == Some(other.clone())) {
        let target = target.expect("proboscis target");
        let position = if start_solid {
            point
        } else {
            sub3(point, scale3(normalize3(sub3(body.origin, point)), 12.0))
        };
        if let Some(state) = game.monsters.states.get_mut(&owner) {
            state.next_frame = parasite_frame::DRAIN06;
        }
        let entity = game.require_entity_mut(&tip);
        entity.style = 1;
        entity.pos1 = sub3(position, target.origin);
        entity.enemy = Some(other.clone());
        entity.render_flags |= 32;
        game.set_motion_kind(tip.clone(), Q2MotionKind::Stationary);
        game.set_solid(tip.clone(), Q2Solid::None);
        game.sound(&tip, "parasite/paratck3.wav", 1, 1.0, 1.0);
        position
    } else {
        let position = add3(point, normal);
        let flagged = game.entity(&other).is_some_and(|target| target.server_flags & 2 != 0);
        if game.host.is_monster(&other) || flagged {
            proboscis_retract(tip.clone(), game);
        } else {
            {
                let mut context = MonsterContext::new(owner.clone(), game);
                context.set_move("parasite_move_break", true);
            }
            game.require_entity_mut(&tip).style = 1;
            game.set_motion_kind(tip.clone(), Q2MotionKind::Stationary);
            game.set_solid(tip.clone(), Q2Solid::None);
            let mut moved = game.body_of(owner.clone());
            moved.angles.y = body.angles.y;
            game.write_body(owner, &moved, true);
        }
        position
    };
    let damageable = game
        .host
        .combat()
        .read(&other)
        .is_some_and(|state| state.can_take_damage);
    if damageable {
        let attacker = game.require_entity(&tip).owner.clone();
        game.damage(
            other,
            tip.clone(),
            attacker,
            5.0,
            0.0,
            normal,
            point,
            normal,
            0,
            0,
            None,
        );
    }
    let owner_actor = game.require_entity(&tip).owner.clone().expect("proboscis owner");
    game.host_emit(Q2PresentationEvent::Sound(Q2SoundEvent {
        actor: Some(owner_actor),
        origin: point,
        path: "parasite/paratck2.wav".to_string(),
        channel: 0,
        volume: 1.0,
        attenuation: 1.0,
        reliable: false,
        loop_: Q2SoundLoop::Once,
        loop_owner: None,
    }));
    let mut moved = game.body_of(tip.clone());
    moved.origin = position;
    game.write_body(tip.clone(), &moved, true);
    let frame_seconds = game.host.frame_seconds();
    game.schedule(tip, frame_seconds, proboscis_think);
}

/// Touch (`touch`).
fn proboscis_touch(tip: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    let origin = game.body_of(tip.clone()).origin;
    let normal = contact.plane.map(|plane| plane.normal).unwrap_or(vec3(0.0, 0.0, 0.0));
    proboscis_hit(tip, game, contact.other, origin, normal, false);
}

/// Think (`think`).
fn proboscis_think(tip: ActorId, game: &mut Q2GameServices) {
    let owner = game.require_entity(&tip).owner.clone();
    let Some(owner) = owner.filter(|owner| game.monsters.states.contains_key(owner)) else {
        proboscis_reset(tip, game);
        return;
    };
    let frame_seconds = game.host.frame_seconds();
    game.schedule(tip.clone(), frame_seconds, proboscis_think);
    let body = game.body_of(tip.clone());
    let style = game.require_entity(&tip).style;
    if style == 2 {
        let origin = proboscis_start_for(game, &owner);
        let direction = sub3(body.origin, origin);
        let distance = length3(direction);
        let speed = game.require_entity(&tip).speed;
        if distance <= speed as f32 * 2.0 * frame_seconds as f32 {
            let entity = game.require_entity_mut(&tip);
            entity.style = 3;
            entity.think = Some(proboscis_reset);
            let mut moved = game.body_of(tip.clone());
            moved.origin = origin;
            game.write_body(tip, &moved, true);
            return;
        }
        let mut moved = game.body_of(tip.clone());
        moved.origin = sub3(
            body.origin,
            scale3(normalize3(direction), speed as f32 * frame_seconds as f32),
        );
        game.write_body(tip, &moved, true);
        return;
    }
    let enemy = game.require_entity(&tip).enemy.clone();
    if style == 1 && enemy.is_some() {
        let enemy = enemy.expect("proboscis enemy");
        let target = game.host.bodies().read(&enemy);
        let combat = game.host.combat().read(&enemy);
        let drainable = match (&target, &combat) {
            (Some(_), Some(combat)) => combat.health > 0.0 && combat.can_take_damage,
            _ => false,
        };
        if !drainable {
            proboscis_retract(tip, game);
            return;
        }
        let target = target.expect("proboscis target");
        let pos1 = game.require_entity(&tip).pos1;
        let origin = add3(target.origin, pos1);
        let from = proboscis_start_for(game, &owner);
        let trace = game.host.trace(&Q2TraceRequest {
            start: from,
            end: origin,
            bounds: None,
            ignore: None,
            mask: 3,
            exclude: Vec::new(),
        });
        let mut moved = game.body_of(tip.clone());
        moved.origin = origin;
        moved.angles = vector_angles(normalize3(sub3(origin, from)));
        game.write_body(tip.clone(), &moved, true);
        if trace.fraction != 1.0 {
            proboscis_retract(tip.clone(), game);
            let mut moved = game.body_of(tip.clone());
            moved.origin = body.origin;
            game.write_body(tip, &moved, true);
            return;
        }
        if game.require_entity(&tip).timestamp <= game.host.now() {
            let normal = match &trace.contact {
                TraceContact::Plane { plane } => plane.normal,
                TraceContact::None => vec3(0.0, 0.0, 0.0),
            };
            let attacker = game.require_entity(&tip).owner.clone();
            game.damage(
                enemy.clone(),
                tip.clone(),
                attacker,
                2.0,
                0.0,
                normal,
                trace.end,
                normal,
                0,
                0,
                None,
            );
            let max_health = game.require_entity(&owner).max_health;
            let hp = max_health.min(health(game, Some(&owner)) + 2.0);
            let owned = game.owned_of(owner.clone());
            game.host.combat().set_health(&owned, hp);
            let entity = game.require_entity_mut(&owner);
            entity.skin = if hp < entity.max_health / 2.0 { 1 } else { 0 };
            let now = game.host.now();
            game.require_entity_mut(&tip).timestamp = now + 0.1;
        }
        game.link_actor(tip);
        return;
    }
    if style == 0 {
        let owner_enemy = game.require_entity(&owner).enemy.clone();
        let target = owner_enemy.as_ref().and_then(|enemy| game.host.bodies().read(enemy));
        if target.is_none() || health(game, owner_enemy.as_ref()) <= 0.0 {
            proboscis_retract(tip, game);
            return;
        }
        let target = target.expect("proboscis target");
        let delta = sub3(body.origin, target.origin);
        let speed = game.require_entity(&tip).speed;
        let owner_origin = game.body_of(owner).origin;
        if length3(delta) > speed as f32 * 2.0 / 15.0
            && dot3(normalize3(delta), normalize3(sub3(body.origin, owner_origin))) > 0.0
        {
            proboscis_retract(tip, game);
        }
    }
}

/// Draw (`draw`).
pub fn proboscis_draw(segment: ActorId, game: &mut Q2GameServices) {
    let owner = game.require_entity(&segment).owner.clone();
    let tip = owner.as_ref().and_then(|tip| game.entity(tip));
    let owner = tip.and_then(|tip| tip.owner.clone());
    let Some(owner) = owner.filter(|owner| game.monsters.states.contains_key(owner)) else {
        return;
    };
    let tip = game.require_entity(&segment).owner.clone().expect("proboscis tip");
    let from = proboscis_start_for(game, &owner);
    let tip_origin = game.body_of(tip).origin;
    let to = sub3(tip_origin, scale3(normalize3(sub3(tip_origin, from)), 8.0));
    game.require_entity_mut(&segment).pos2 = to;
    let mut moved = game.body_of(segment.clone());
    moved.origin = from;
    game.write_body(segment, &moved, true);
}

/// Fire (`fire`).
pub fn proboscis_fire(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let previous = context.game.require_entity(&actor).proboscus.clone();
    if let Some(previous) = previous {
        if context.game.require_entity(&previous).style != 2 {
            proboscis_reset(previous, &mut *context.game);
        }
    }
    let from = proboscis_start(context);
    let offset = f64::from(rerelease_random(context).float_range(-0.999_999_940_395_355_2, 1.0) * 0.1f32);
    let Some(direction) = predicted_direction(context, from, 1250.0, false, offset) else {
        return;
    };
    let tip = context
        .game
        .create("parasite_proboscis", std::collections::BTreeMap::new());
    let segment = context
        .game
        .create("parasite_proboscis_segment", std::collections::BTreeMap::new());
    let entity = context.game.require_entity_mut(&tip);
    entity.model = "models/monsters/parasite/tip/tris.md2".to_string();
    entity.owner = Some(actor.clone());
    context.game.require_entity_mut(&actor).proboscus = Some(tip.clone());
    let entity = context.game.require_entity_mut(&tip);
    entity.clip_mask = 3 | 0x200_0000 | 0x4000_0000;
    entity.speed = 1250.0;
    entity.projectile = true;
    entity.die = Some(proboscis_die);
    entity.touch = Some(proboscis_touch);
    entity.flags |= 8;
    let owned = context.game.owned_of(tip.clone());
    context.game.create_combat(&owned, 0.0, 0.0, true);
    let mut moved = context.game.body_of(tip.clone());
    moved.origin = from;
    moved.angles = vector_angles(direction);
    moved.velocity = scale3(direction, 1250.0);
    context.game.write_body(tip.clone(), &moved, true);
    context.game.set_motion_kind(tip.clone(), Q2MotionKind::FlyMissile);
    context.game.set_solid(tip.clone(), Q2Solid::Box);
    let frame_seconds = context.game.host.frame_seconds();
    context.game.schedule(tip.clone(), frame_seconds, proboscis_think);
    let entity = context.game.require_entity_mut(&segment);
    entity.model = "models/monsters/parasite/segment/tris.md2".to_string();
    entity.render_flags = 128;
    entity.postthink = Some(proboscis_draw);
    context.game.require_entity_mut(&tip).proboscus = Some(segment.clone());
    context.game.require_entity_mut(&segment).owner = Some(tip.clone());
    let frame_seconds = context.game.host.frame_seconds();
    let trace = context.game.host.trace(&Q2TraceRequest {
        start: from,
        end: add3(from, scale3(direction, 1250.0 * frame_seconds as f32)),
        bounds: None,
        ignore: Some(actor.clone()),
        mask: 3 | 0x200_0000 | 0x4000_0000,
        exclude: Vec::new(),
    });
    if trace.start_solid || trace.fraction < 1.0 {
        let other = match &trace.hit {
            TraceHit::Actor { actor } => actor.clone(),
            _ => context.game.host.world_actor(),
        };
        let point = if trace.start_solid { from } else { trace.end };
        let normal = if trace.start_solid {
            scale3(direction, -1.0)
        } else {
            match &trace.contact {
                TraceContact::Plane { plane } => plane.normal,
                TraceContact::None => vec3(0.0, 0.0, 0.0),
            }
        };
        let start_solid = trace.start_solid;
        proboscis_hit(tip.clone(), &mut *context.game, other, point, normal, start_solid);
    }
    let tip_origin = context.game.body_of(tip.clone()).origin;
    context.game.require_entity_mut(&segment).pos2 = add3(tip_origin, scale3(normalize3(sub3(tip_origin, from)), 8.0));
    let mut moved = context.game.body_of(segment.clone());
    moved.origin = from;
    context.game.write_body(segment.clone(), &moved, true);
    context.game.show(tip);
    context.game.show(segment);
}

/// Proboscis callbacks (`callbacks`).
pub fn proboscis_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks
        .think
        .insert("rerelease.parasite.proboscis_reset", proboscis_reset);
    callbacks
        .think
        .insert("rerelease.parasite.proboscis_think", proboscis_think);
    callbacks
        .think
        .insert("rerelease.parasite.proboscis_segment_draw", proboscis_draw);
    callbacks
        .touch
        .insert("rerelease.parasite.proboscis_touch", proboscis_touch);
    callbacks.die.insert("rerelease.parasite.proboscis_die", proboscis_die);
    callbacks
}
