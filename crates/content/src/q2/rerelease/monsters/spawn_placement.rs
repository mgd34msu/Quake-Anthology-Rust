//! Rerelease spawn placement (`src/content/q2/rerelease/monsters/spawn-placement.ts`).
//!
//! Quake II rerelease rogue/g_rogue_spawn.cpp, g_monster.cpp, m_move.cpp.
//! Copyright (c) ZeniMax Media Inc. GPL-2.0-or-later.

use qa_core::math::{Bounds, Vec3, vec3};
use qa_core::numeric::{NumericOps, Q3_BINARY32_PROFILE};

use crate::q2::foundation::host::{Q2GameServices, Q2TraceRequest};
use crate::q2::foundation::monsters::ai::monster_solid_mask;
use crate::q2::missionpacks::monsters::spawn::check_rogue_spawn_point;
use crate::q2::support::contracts::TraceFamily;
use crate::q2::support::movement::{
    MovementVector, StuckResult, StuckTrace, fix_stuck_object,
};

/// Movement vector (`movementVector`).
fn movement_vector(v: Vec3) -> MovementVector {
    [f64::from(v.x), f64::from(v.y), f64::from(v.z)]
}

/// Arena vector (`vector`).
fn arena_vector(v: MovementVector) -> Vec3 {
    vec3(v[0] as f32, v[1] as f32, v[2] as f32)
}

/// Drop to floor (`dropToFloor`).
fn drop_to_floor(game: &mut Q2GameServices, origin: Vec3, bounds: Bounds) -> Option<Vec3> {
    let mask = monster_solid_mask(game);
    let mut origin = origin;
    let probe = game.host.trace(&Q2TraceRequest {
        start: origin,
        end: origin,
        bounds: Some(bounds),
        ignore: None,
        mask,
        exclude: Vec::new(),
    });
    if probe.start_solid {
        origin.z += 1.0;
    }
    let trace = game.host.trace(&Q2TraceRequest {
        start: origin,
        end: vec3(origin.x, origin.y, origin.z - 256.0),
        bounds: Some(bounds),
        ignore: None,
        mask,
        exclude: Vec::new(),
    });
    if trace.fraction == 1.0 || trace.all_solid || trace.start_solid {
        None
    } else {
        Some(trace.end)
    }
}

/// Find a rerelease spawn point (`findRereleaseSpawnPoint`).
pub fn find_rerelease_spawn_point(
    game: &mut Q2GameServices,
    start: Vec3,
    bounds: Bounds,
    _max_move_up: f64,
    drop: bool,
) -> Option<Vec3> {
    if drop {
        if let Some(dropped) = drop_to_floor(game, start, bounds) {
            return Some(dropped);
        }
    }
    let ops = NumericOps::select(Q3_BINARY32_PROFILE).expect("binary32 profile");
    let mut origin = movement_vector(start);
    let mins = movement_vector(bounds.min);
    let maxs = movement_vector(bounds.max);
    let result = fix_stuck_object(
        &ops,
        &mut origin,
        &mins,
        &maxs,
        &mut |from, mins, maxs, end| {
            let trace = game.host.trace(&Q2TraceRequest {
                start: arena_vector(*from),
                end: arena_vector(*end),
                bounds: Some(Bounds {
                    min: arena_vector(*mins),
                    max: arena_vector(*maxs),
                }),
                ignore: None,
                mask: monster_solid_mask(game),
                exclude: Vec::new(),
            });
            if !matches!(trace.family, TraceFamily::Q2(_)) {
                panic!("Rerelease monster placement requires Q2 trace fields");
            }
            StuckTrace {
                start_solid: trace.start_solid,
                endpos: movement_vector(trace.end),
            }
        },
    );
    if result == StuckResult::NoGoodPosition {
        return None;
    }
    if drop {
        drop_to_floor(game, arena_vector(origin), bounds)
    } else {
        Some(arena_vector(origin))
    }
}

/// Check a rerelease ground spawn point (`checkRereleaseGroundSpawnPoint`).
pub fn check_rerelease_ground_spawn_point(
    game: &mut Q2GameServices,
    origin: Vec3,
    bounds: Bounds,
    _height: f64,
    _gravity: f64,
) -> bool {
    if !check_rogue_spawn_point(game, origin, bounds) {
        return false;
    }
    let bottom = origin.z + bounds.min.z;
    let mut fast = true;
    for x in [origin.x + bounds.min.x, origin.x + bounds.max.x] {
        for y in [origin.y + bounds.min.y, origin.y + bounds.max.y] {
            if game.host.point_contents(vec3(x, y, bottom - 1.0)) != 1 {
                fast = false;
                break;
            }
        }
        if !fast {
            break;
        }
    }
    if fast {
        return true;
    }
    let mask = monster_solid_mask(game);
    let start = vec3(origin.x, origin.y, bottom);
    let stop_z = bottom - 36.0;
    let trace = game.host.trace(&Q2TraceRequest {
        start,
        end: vec3(start.x, start.y, stop_z),
        bounds: Some(Bounds {
            min: vec3(bounds.min.x, bounds.min.y, 0.0),
            max: vec3(bounds.max.x, bounds.max.y, 0.0),
        }),
        ignore: None,
        mask,
        exclude: Vec::new(),
    });
    if trace.fraction == 1.0 {
        return false;
    }
    let center = vec3(
        origin.x + (bounds.min.x + bounds.max.x) * 0.5,
        origin.y + (bounds.min.y + bounds.max.y) * 0.5,
        bottom,
    );
    let half = vec3((bounds.max.x - bounds.min.x) * 0.5 * 0.5, (bounds.max.y - bounds.min.y) * 0.5 * 0.5, 0.0);
    let quadrant_bounds = Bounds {
        min: vec3(-half.x, -half.y, 0.0),
        max: half,
    };
    for x in [center.x - half.x, center.x + half.x] {
        for y in [center.y - half.y, center.y + half.y] {
            let corner = game.host.trace(&Q2TraceRequest {
                start: vec3(x, y, bottom),
                end: vec3(x, y, stop_z),
                bounds: Some(quadrant_bounds),
                ignore: None,
                mask,
                exclude: Vec::new(),
            });
            if corner.fraction == 1.0 || trace.end.z - corner.end.z > 18.0 {
                return false;
            }
        }
    }
    true
}
