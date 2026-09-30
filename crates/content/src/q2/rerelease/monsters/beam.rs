//! Rerelease monster beam (`src/content/q2/rerelease/monsters/beam.ts`).
//!
//! ZeniMax Media, GPL-2.0-or-later.

use std::collections::BTreeMap;

use qa_core::identity::ActorId;
use qa_core::math::{Vec3, add3, scale3, vec3};

use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::host::{
    Q2BeamEvent, Q2EffectEvent, Q2GameServices, Q2MotionKind,
    Q2PresentationEvent, Q2Solid, Q2SoundEvent, Q2SoundLoop, Q2Think,
    Q2TraceRequest,
};
use crate::q2::foundation::monsters::types::MonsterContext;
use crate::q2::support::contracts::{TraceContact, TraceHit};

/// Update a monster beam (`updateMonsterBeam`).
pub fn update_monster_beam(beam: &ActorId, game: &mut Q2GameServices, damage: bool) {
    let start = game.body_of(beam.clone()).origin;
    let movedir = game.require_entity(beam).movedir;
    let end = add3(start, scale3(movedir, 2048.0));
    let mut exclude: Vec<ActorId> = Vec::new();
    let mut endpoint = end;
    loop {
        let trace = game.host.trace(&Q2TraceRequest {
            start,
            end,
            bounds: None,
            ignore: Some(beam.clone()),
            exclude: exclude.clone(),
            mask: 3 | 0x2000000 | 0x4000000 | 0x40000000,
        });
        let normal = match &trace.contact {
            TraceContact::Plane { plane } => plane.normal,
            TraceContact::None => vec3(0.0, 0.0, 0.0),
        };
        endpoint = add3(trace.end, normal);
        let TraceHit::Actor { actor: target } = &trace.hit else {
            break;
        };
        let target = target.clone();
        if exclude.contains(&target) {
            break;
        }
        let (beam_damage, beam_owner) = {
            let entity = game.require_entity(beam);
            (entity.damage, entity.owner.clone())
        };
        if damage
            && beam_damage > 0.0
            && game.host.combat().read(&target).is_some_and(|state| state.can_take_damage)
            && game.entity(&target).is_some_and(|entity| !entity.laser_immune)
            && beam_owner.as_ref() != Some(&target)
        {
            game.damage(
                target.clone(),
                beam.clone(),
                beam_owner,
                beam_damage,
                f64::from(game.options.skill),
                movedir,
                trace.end,
                vec3(0.0, 0.0, 0.0),
                30,
                4,
                None,
            );
        } else if damage && beam_damage < 0.0 {
            let combat = game.host.combat().read(&target);
            let entity = game.entity(&target).cloned();
            if let (Some(combat), Some(entity)) = (combat, entity) {
                if combat.health < entity.max_health {
                    game.host.combat().set_health(
                        &entity.actor,
                        entity.max_health.min(combat.health - beam_damage),
                    );
                }
            }
        }
        if !game.host.is_monster(&target) && !game.host.is_player(&target) {
            if damage {
                let skin = game.require_entity(beam).skin;
                game.host_emit(Q2PresentationEvent::Effect(Q2EffectEvent {
                    effect: "q2:laser-sparks".to_string(),
                    origin: trace.end,
                    direction: normal,
                    count: 10,
                    color: skin,
                }));
            }
            break;
        }
        if exclude.len() == 16 {
            break;
        }
        exclude.push(target);
    }
    game.require_entity_mut(beam).pos2 = endpoint;
    let (frame, skin) = {
        let entity = game.require_entity(beam);
        (entity.frame, entity.skin)
    };
    game.host_emit(Q2PresentationEvent::Beam(Q2BeamEvent {
        actor: beam.clone(),
        start,
        end: endpoint,
        width: f64::from(frame),
        color: skin,
        visible: true,
    }));
}

/// Free a monster beam (`freeMonsterBeam`).
pub fn free_monster_beam(beam: ActorId, game: &mut Q2GameServices) {
    let owner = game.require_entity(&beam).owner.clone();
    if let Some(owner_id) = owner {
        if game.entity(&owner_id).is_some() {
            if game.require_entity(&beam).spawnflags & 1 != 0 {
                game.require_entity_mut(&owner_id).beam2 = None;
            } else {
                game.require_entity_mut(&owner_id).beam = None;
            }
        }
    }
    let origin = game.body_of(beam.clone()).origin;
    let (pos2, frame, skin) = {
        let entity = game.require_entity(&beam);
        (entity.pos2, entity.frame, entity.skin)
    };
    game.host_emit(Q2PresentationEvent::Beam(Q2BeamEvent {
        actor: beam.clone(),
        start: origin,
        end: pos2,
        width: f64::from(frame),
        color: skin,
        visible: false,
    }));
    game.remove_actor(beam);
}

/// Fire a monster beam (`fireMonsterBeam`).
pub fn fire_monster_beam(
    context: &mut MonsterContext,
    damage: f64,
    secondary: bool,
    update: Q2Think,
) {
    let actor = context.actor().clone();
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks.think.insert("beam_think", free_monster_beam);
    context.game.source_callbacks.register(&callbacks);
    let existing = if secondary {
        context.entity().beam2.clone()
    } else {
        context.entity().beam.clone()
    };
    let beam = match existing.as_ref().and_then(|id| context.game.entity(id)) {
        Some(_) => existing.expect("monster beam id"),
        None => {
            let beam = context.game.create("dabeam", BTreeMap::new());
            if secondary {
                context.entity_mut().beam2 = Some(beam.clone());
            } else {
                context.entity_mut().beam = Some(beam.clone());
            }
            let medic = context.state().medic;
            {
                let entity = context.game.require_entity_mut(&beam);
                entity.owner = Some(actor);
                entity.damage = damage;
                entity.frame = 2;
                entity.spawnflags = if secondary { 1 } else { 0 };
                entity.skin = if medic { 0xf3f3f1f1u32 as i32 } else { 0xf2f2f0f0u32 as i32 };
                entity.render_flags |= 128;
                entity.postthink = Some(update);
                entity.sound = "misc/lasfly.wav".to_string();
            }
            context.game.set_motion_kind(beam.clone(), Q2MotionKind::Stationary);
            context.game.set_solid(beam.clone(), Q2Solid::None);
            let origin = context.game.body_of(beam.clone()).origin;
            context.game.host_emit(Q2PresentationEvent::Sound(Q2SoundEvent {
                actor: Some(beam.clone()),
                origin,
                path: "misc/lasfly.wav".to_string(),
                channel: 0,
                volume: 1.0,
                attenuation: 1.0,
                reliable: false,
                loop_: Q2SoundLoop::Start,
                loop_owner: None,
            }));
            beam
        }
    };
    context.game.schedule(beam.clone(), 0.2, free_monster_beam);
    update(beam.clone(), &mut *context.game);
    update_monster_beam(&beam, &mut *context.game, true);
}
