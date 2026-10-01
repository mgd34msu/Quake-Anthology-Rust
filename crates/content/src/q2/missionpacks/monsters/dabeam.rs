//! Transient monster beam (`src/content/q2/missionpacks/monsters/dabeam.ts`).
//!
//! Quake II xatrix/g_monster.c transient monster healing/laser beam.
//! GPL-2.0-or-later.

use std::collections::BTreeMap;

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3, add3, normalize3, scale3, sub3, vec3};

use crate::q2::foundation::callbacks::{Q2CallbackDefinitions, free_q2_entity};
use crate::q2::foundation::host::{
    Q2BeamEvent, Q2EffectEvent, Q2GameServices, Q2MotionKind, Q2PresentationEvent, Q2Solid,
    Q2TraceRequest,
};
use crate::q2::foundation::monsters::ai::{angles_vectors, trace_ground_actor};
use crate::q2::support::contracts::{TraceContact, TraceResult};

/// Beam hit think (`beamHit`).
fn beam_hit(entity: ActorId, game: &mut Q2GameServices) {
    let origin = game.body_of(entity.clone()).origin;
    let movedir = game.require_entity(&entity).movedir;
    let end = add3(origin, scale3(movedir, 2048.0));
    let mut start = origin;
    let mut ignore = entity.clone();
    let mut endpoint = end;
    loop {
        let trace: TraceResult = game.host.trace(&Q2TraceRequest {
            start,
            end,
            bounds: None,
            ignore: Some(ignore.clone()),
            mask: 1 | 0x2000000 | 0x4000000,
            exclude: Vec::new(),
        });
        endpoint = trace.end;
        let actor = trace_ground_actor(&trace, game);
        let Some(actor) = actor else { break };
        let damageable = game
            .host
            .combat()
            .read(&actor)
            .is_some_and(|state| state.can_take_damage);
        let target = game.entities.get(&actor).cloned();
        let owner = game.require_entity(&entity).owner.clone();
        if damageable
            && target.as_ref().is_some_and(|target| !target.laser_immune)
            && target.as_ref().map(|target| target.flags).unwrap_or(0) & 4 == 0
            && Some(&actor) != owner.as_ref()
        {
            let damage = game.require_entity(&entity).damage;
            let skill = game.options.skill;
            game.damage(
                actor.clone(),
                entity.clone(),
                owner,
                damage,
                f64::from(skill),
                movedir,
                trace.end,
                vec3(0.0, 0.0, 0.0),
                30,
                4,
                None,
            );
        }
        let damage = game.require_entity(&entity).damage;
        let updated = game.host.combat().read(&actor);
        if damage < 0.0
            && game.host.is_player(&actor)
            && updated.as_ref().is_some_and(|state| state.health > 100.0)
        {
            let updated = updated.expect("beam heal target");
            if let Some(owned) = game.host.actors().resolve_owned(&actor) {
                game.host.combat().set_health(&owned, updated.health + damage);
            }
        }
        if !game.host.is_monster(&actor) && !game.host.is_player(&actor) {
            if game.require_entity(&entity).spawnflags & (0x80000000u32 as i32) != 0 {
                game.require_entity_mut(&entity).spawnflags &= !(0x80000000u32 as i32);
                let direction = match &trace.contact {
                    TraceContact::Plane { plane } => plane.normal,
                    TraceContact::None => vec3(0.0, 0.0, 0.0),
                };
                let color = game.require_entity(&entity).skin & 255;
                game.host_emit(Q2PresentationEvent::Effect(Q2EffectEvent {
                    effect: "q2:laser-sparks".to_string(),
                    origin: trace.end,
                    direction,
                    count: 10,
                    color,
                }));
            }
            break;
        }
        ignore = actor;
        start = trace.end;
    }
    let skin = game.require_entity(&entity).skin;
    game.host_emit(Q2PresentationEvent::Beam(Q2BeamEvent {
        actor: entity.clone(),
        start: origin,
        end: endpoint,
        width: 2.0,
        color: skin,
        visible: true,
    }));
    game.schedule(entity, 0.1, free_q2_entity);
}

/// Dabeam callbacks (`monsterDabeamCallbacks`).
pub fn monster_dabeam_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks.think.insert("dabeam_hit", beam_hit);
    callbacks.think.insert("G_FreeEdict", free_q2_entity);
    callbacks
}

/// Spawn a transient monster beam (`monsterDabeam`).
pub fn monster_dabeam(
    owner: &ActorId,
    game: &mut Q2GameServices,
    target: Option<ActorId>,
    origin: Vec3,
    angles: Vec3,
    damage: f64,
    medic: bool,
) -> ActorId {
    game.source_callbacks.register(&monster_dabeam_callbacks());
    let beam = game.create("dabeam", BTreeMap::new());
    {
        let beam_entity = game.require_entity_mut(&beam);
        beam_entity.owner = Some(owner.clone());
        beam_entity.enemy = target.clone();
        beam_entity.damage = damage;
        beam_entity.render_flags = 128 | 32;
        beam_entity.frame = 2;
        beam_entity.skin = if medic { 0xf3f3f1f1u32 as i32 } else { 0xf2f2f0f0u32 as i32 };
    }
    let enemy = target
        .as_ref()
        .and_then(|target| game.host.bodies().read(target));
    if let Some(enemy) = enemy {
        let point = add3(
            enemy.origin,
            scale3(add3(enemy.bounds.min, enemy.bounds.max), 0.5),
        );
        let aim = if medic {
            vec3(point.x + (game.host.now().sin() * 8.0) as f32, point.y, point.z)
        } else {
            point
        };
        game.require_entity_mut(&beam).movedir = normalize3(sub3(aim, origin));
    } else {
        game.require_entity_mut(&beam).movedir = angles_vectors(angles).forward;
    }
    game.require_entity_mut(&beam).spawnflags |= 0x80000001u32 as i32;
    let mut body = game.body_of(beam.clone());
    body.origin = origin;
    body.angles = angles;
    body.bounds = Bounds {
        min: vec3(-8.0, -8.0, -8.0),
        max: vec3(8.0, 8.0, 8.0),
    };
    game.write_body(beam.clone(), &body, false);
    game.set_motion_kind(beam.clone(), Q2MotionKind::Stationary);
    game.set_solid(beam.clone(), Q2Solid::None);
    game.schedule(beam.clone(), 0.1, beam_hit);
    beam
}
