//! Rogue spawn helpers (`src/content/q2/missionpacks/monsters/spawn.ts`).
//!
//! Original Rogue g_spawn.c monster creation and spawn growth.
//! ZeniMax Media, GPL-2.0-or-later.

use std::collections::BTreeMap;

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3, add3, vec3};

use super::rogue_common::source_trace_world;
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::host::{
    Q2GameServices, Q2MotionKind, Q2Solid, Q2TraceRequest,
};
use crate::q2::foundation::monsters::ai::monster_solid_mask;
use crate::q2::foundation::monsters::spawn_summoned_monster;
use crate::q2::support::contracts::TraceFamily;

/// Find a rogue spawn point (`findRogueSpawnPoint`).
pub fn find_rogue_spawn_point(
    game: &mut Q2GameServices,
    start: Vec3,
    bounds: Bounds,
    max_move_up: f64,
) -> Option<Vec3> {
    let trace = game.host.trace(&Q2TraceRequest {
        start,
        end: start,
        bounds: Some(bounds),
        ignore: None,
        mask: monster_solid_mask(game) | 0x10000,
        exclude: Vec::new(),
    });
    if trace.start_solid || trace.all_solid || !source_trace_world(game, &trace) {
        let raised = vec3(start.x, start.y, start.z + max_move_up as f32);
        let trace = game.host.trace(&Q2TraceRequest {
            start: raised,
            end: start,
            bounds: Some(bounds),
            ignore: None,
            mask: monster_solid_mask(game),
            exclude: Vec::new(),
        });
        return if trace.start_solid || trace.all_solid {
            None
        } else {
            Some(trace.end)
        };
    }
    Some(start)
}

/// Check a rogue spawn point (`checkRogueSpawnPoint`).
pub fn check_rogue_spawn_point(game: &mut Q2GameServices, origin: Vec3, bounds: Bounds) -> bool {
    if bounds.min.x == 0.0 && bounds.min.y == 0.0 && bounds.min.z == 0.0
        || bounds.max.x == 0.0 && bounds.max.y == 0.0 && bounds.max.z == 0.0
    {
        return false;
    }
    let trace = game.host.trace(&Q2TraceRequest {
        start: origin,
        end: origin,
        bounds: Some(bounds),
        ignore: None,
        mask: monster_solid_mask(game),
        exclude: Vec::new(),
    });
    !trace.start_solid && !trace.all_solid && source_trace_world(game, &trace)
}

/// Check a rogue ground spawn point (`checkRogueGroundSpawnPoint`).
pub fn check_rogue_ground_spawn_point(
    game: &mut Q2GameServices,
    origin: Vec3,
    bounds: Bounds,
    height: f64,
    gravity: f64,
) -> bool {
    if !check_rogue_spawn_point(game, origin, bounds) {
        return false;
    }
    let mask = monster_solid_mask(game);
    let mut stop = vec3(origin.x, origin.y, origin.z + bounds.min.z - height as f32);
    let trace = game.host.trace(&Q2TraceRequest {
        start: origin,
        end: stop,
        bounds: Some(bounds),
        ignore: None,
        mask: mask | 56,
        exclude: Vec::new(),
    });
    let contents = match &trace.family {
        TraceFamily::Q2(fields) => Some(fields.contents),
        TraceFamily::Q3 { contents, .. } => Some(*contents),
        TraceFamily::Q1 { .. } => None,
    };
    let Some(contents) = contents else {
        return false;
    };
    if trace.fraction >= 1.0 || contents & mask == 0 {
        return false;
    }
    let min = add3(trace.end, bounds.min);
    let max = add3(trace.end, bounds.max);
    let corners = [
        (min.x, min.y),
        (min.x, max.y),
        (max.x, min.y),
        (max.x, max.y),
    ];
    if corners.iter().all(|(x, y)| {
        game.host.point_contents(vec3(
            *x,
            *y,
            if gravity > 0.0 { max.z + 1.0 } else { min.z - 1.0 },
        )) == 1
    }) {
        return true;
    }
    let mut start = vec3((min.x + max.x) * 0.5, (min.y + max.y) * 0.5, min.z);
    stop = vec3(start.x, start.y, stop.z);
    let trace = game.host.trace(&Q2TraceRequest {
        start,
        end: stop,
        bounds: None,
        ignore: None,
        mask,
        exclude: Vec::new(),
    });
    if trace.fraction == 1.0 {
        return false;
    }
    let mid = trace.end.z + if gravity < 0.0 { bounds.min.z } else { -bounds.max.z };
    start = vec3(start.x, start.y, if gravity < 0.0 { min.z } else { max.z });
    stop = vec3(
        stop.x,
        stop.y,
        start.z + if gravity < 0.0 { -36.0 } else { 36.0 },
    );
    for (x, y) in corners {
        let trace = game.host.trace(&Q2TraceRequest {
            start: vec3(x, y, start.z),
            end: vec3(x, y, stop.z),
            bounds: None,
            ignore: None,
            mask,
            exclude: Vec::new(),
        });
        let drop = if gravity > 0.0 {
            trace.end.z - mid
        } else {
            mid - trace.end.z
        };
        if trace.fraction == 1.0 || drop > 18.0 {
            return false;
        }
    }
    true
}

/// Create a rogue monster (`createRogueMonster`).
pub fn create_rogue_monster(
    game: &mut Q2GameServices,
    origin: Vec3,
    angles: Vec3,
    classname: &str,
) -> ActorId {
    let actor = game.create(classname, BTreeMap::new());
    let mut moved = game.body_of(actor.clone());
    moved.origin = origin;
    moved.angles = angles;
    game.write_body(actor.clone(), &moved, false);
    game.require_entity_mut(&actor).gravity_vector = vec3(0.0, 0.0, -1.0);
    spawn_summoned_monster(game, actor.clone());
    game.require_entity_mut(&actor).render_flags |= 32768;
    actor
}

/// Create a rogue ground monster (`createRogueGroundMonster`).
pub fn create_rogue_ground_monster(
    game: &mut Q2GameServices,
    origin: Vec3,
    angles: Vec3,
    bounds: Bounds,
    classname: &str,
    height: f64,
) -> Option<ActorId> {
    if check_rogue_ground_spawn_point(game, origin, bounds, height, -1.0) {
        Some(create_rogue_monster(game, origin, angles, classname))
    } else {
        None
    }
}

/// Random angles (`randomAngles`).
fn random_angles(game: &mut Q2GameServices) -> Vec3 {
    let mut angles = vec3(0.0, 0.0, 0.0);
    for _ in 0..2 {
        angles = vec3(
            (game.random() * 360.0).floor() as f32,
            (game.random() * 360.0).floor() as f32,
            (game.random() * 360.0).floor() as f32,
        );
    }
    angles
}

/// Spawn growth think (`spawnGrowThink`).
fn spawn_grow_think(actor: ActorId, game: &mut Q2GameServices) {
    let angles = random_angles(game);
    let mut moved = game.body_of(actor.clone());
    moved.angles = angles;
    game.write_body(actor.clone(), &moved, true);
    let now = game.host.now();
    let (wait, frame, effects, next_think) = {
        let entity = game.require_entity(&actor);
        (entity.wait, entity.frame, entity.effects, entity.next_think)
    };
    if now < wait && frame < 2 {
        game.require_entity_mut(&actor).frame += 1;
    }
    if now >= wait {
        if effects & 0x10000000 != 0 || frame == 0 {
            game.remove_actor(actor);
            return;
        }
        game.require_entity_mut(&actor).frame -= 1;
    }
    game.show(actor.clone());
    let base = next_think.unwrap_or(now);
    game.schedule(actor, base + 0.1 - now, spawn_grow_think);
}

/// Rogue spawn callbacks (`rogueSpawnCallbacks`).
pub fn rogue_spawn_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks.think.insert("q2:rogue/spawngrow_think", spawn_grow_think);
    callbacks
}

/// Spawn a growing portal (`rogueSpawnGrow`).
pub fn rogue_spawn_grow(game: &mut Q2GameServices, origin: Vec3, size: i32) -> ActorId {
    game.source_callbacks.register(&rogue_spawn_callbacks());
    let actor = game.create("spawngro", BTreeMap::new());
    let angles = random_angles(game);
    let mut moved = game.body_of(actor.clone());
    moved.origin = origin;
    moved.angles = angles;
    game.write_body(actor.clone(), &moved, false);
    let now = game.host.now();
    {
        let entity = game.require_entity_mut(&actor);
        entity.render_flags = 32768;
        entity.model = if size <= 1 {
            "models/items/spawngro2/tris.md2"
        } else if size == 2 {
            "models/items/spawngro3/tris.md2"
        } else {
            "models/items/spawngro/tris.md2"
        }
        .to_string();
        entity.wait = now + if size == 2 { 2.0 } else { 0.3 };
        if size != 2 {
            entity.effects |= 0x10000000;
        }
    }
    game.set_solid(actor.clone(), Q2Solid::None);
    game.set_motion_kind(actor.clone(), Q2MotionKind::Stationary);
    game.schedule(actor.clone(), 0.1, spawn_grow_think);
    game.link_actor(actor.clone());
    game.show(actor.clone());
    actor
}
