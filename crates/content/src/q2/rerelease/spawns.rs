//! Q2 rerelease spawns (`src/content/q2/rerelease/spawns.ts`).
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

use qa_core::identity::ActorId;
use qa_core::math::{add3, scale3, vec3, Bounds, Vec3};

use super::types::{q2_is_n64, Q2RereleaseOptions};
use crate::q2::base::player::landmarks::fix_q2_stuck_player;
use crate::q2::base::player::spawns::{q2_entities_named, q2_players_range};
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::host::{Q2GameServices, Q2Mode, Q2MotionKind, Q2Solid, Q2TraceRequest};
use crate::q2::support::contracts::TraceHit;

/// Rerelease player bounds (`q2RereleasePlayerBounds`).
pub fn q2_rerelease_player_bounds() -> Bounds {
    Bounds {
        min: vec3(-16.0, -16.0, -24.0),
        max: vec3(16.0, 16.0, 32.0),
    }
}

/// Drop an N64 start into the world (`startDrop`).
fn start_drop(entity: ActorId, game: &mut Q2GameServices) {
    let mut body = game.body_of(entity.clone());
    body.bounds = q2_rerelease_player_bounds();
    game.write_body(entity.clone(), &body, false);
    game.set_solid(entity.clone(), Q2Solid::Trigger);
    game.set_motion_kind(entity.clone(), Q2MotionKind::Toss);
    game.link_actor(entity);
}

/// Rerelease spawn callbacks (`q2RereleaseSpawns` callbacks).
pub fn q2_rerelease_spawns_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks.think.insert("rr.info_player_start_drop", start_drop);
    callbacks
}

/// Claim rerelease spawn points (`q2RereleaseSpawns` spawn).
pub fn q2_rerelease_spawns_spawn(entity: ActorId, game: &mut Q2GameServices) -> bool {
    let classname = game.require_entity(&entity).classname.clone();
    match classname.as_str() {
        "info_player_start"
        | "info_player_coop"
        | "info_player_coop_lava"
        | "info_player_intermission"
        | "info_player_deathmatch" => {}
        _ => return false,
    }
    if ((classname == "info_player_coop" || classname == "info_player_coop_lava") && game.options.mode != Q2Mode::Coop)
        || (classname == "info_player_deathmatch" && game.options.mode != Q2Mode::Deathmatch)
    {
        game.remove_actor(entity);
        return true;
    }
    if classname == "info_player_intermission" {
        return true;
    }
    if classname == "info_player_deathmatch" {
        {
            let record = game.require_entity_mut(&entity);
            record.model = "models/objects/dmspot/tris.md2".to_string();
            record.skin = 1;
        }
        let mut body = game.body_of(entity.clone());
        body.bounds = Bounds {
            min: vec3(-32.0, -32.0, -24.0),
            max: vec3(32.0, 32.0, -16.0),
        };
        game.write_body(entity.clone(), &body, false);
        game.set_solid(entity.clone(), Q2Solid::Box);
        game.show(entity);
        return true;
    }
    let origin = game.body_of(entity.clone()).origin;
    let stuck = game.host.trace(&Q2TraceRequest {
        start: origin,
        end: origin,
        bounds: Some(q2_rerelease_player_bounds()),
        ignore: Some(entity.clone()),
        mask: 1,
        exclude: Vec::new(),
    });
    if stuck.start_solid {
        if let Some(fixed) = fix_q2_stuck_player(entity.clone(), game, origin, q2_rerelease_player_bounds()) {
            let mut body = game.body_of(entity.clone());
            body.origin = fixed;
            game.write_body(entity.clone(), &body, true);
        }
    }
    if classname != "info_player_coop_lava" && q2_is_n64(game) {
        let frame = game.host.frame_seconds();
        game.schedule(entity, frame, start_drop);
    }
    true
}

/// Find the single-player start (`q2RereleaseSingleSpawn`).
pub fn q2_rerelease_single_spawn(game: &mut Q2GameServices, spawn_point: &str) -> Option<ActorId> {
    let starts = q2_entities_named(game, "info_player_start");
    if let Some(spot) = starts
        .iter()
        .find(|spot| game.require_entity(spot).targetname.to_lowercase() == spawn_point.to_lowercase())
    {
        return Some(spot.clone());
    }
    if let Some(spot) = starts
        .iter()
        .find(|spot| game.require_entity(spot).targetname.is_empty())
    {
        return Some(spot.clone());
    }
    starts.first().cloned()
}

/// Find the lava coop spawn (`lavaSpawn`).
fn lava_spawn(game: &mut Q2GameServices) -> Option<ActorId> {
    let mut lava_top: f64 = -99999.0;
    for water in q2_entities_named(game, "func_water") {
        let body = game.body_of(water.clone());
        let center = add3(body.origin, scale3(add3(body.bounds.min, body.bounds.max), 0.5));
        let flags = game.require_entity(&water).spawnflags;
        if flags & 2 != 0 && game.host.point_contents(center) & 56 != 0 {
            lava_top = lava_top.max(f64::from(body.origin.z + body.bounds.max.z));
        }
    }
    if lava_top == -99999.0 {
        return None;
    }
    let mut best: Option<ActorId> = None;
    let mut height = 999999.0f64;
    for spot in q2_entities_named(game, "info_player_coop_lava").into_iter().take(64) {
        let z = f64::from(game.body_of(spot.clone()).origin.z);
        if z >= lava_top + 64.0 && z < height && q2_players_range(game, spot.clone()) > 32.0 {
            height = z;
            best = Some(spot);
        }
    }
    best
}

/// Whether a deathmatch spot is clear of spawns.
fn spawn_clear(game: &mut Q2GameServices, spot: &ActorId, bounds: Bounds) -> bool {
    let origin = add3(game.body_of(spot.clone()).origin, vec3(0.0, 0.0, 9.0));
    !game
        .host
        .trace(&Q2TraceRequest {
            start: origin,
            end: origin,
            bounds: Some(bounds),
            ignore: Some(spot.clone()),
            mask: 0x42000000,
            exclude: Vec::new(),
        })
        .start_solid
}

/// Select a deathmatch spawn.
fn deathmatch_spawn(
    game: &mut Q2GameServices,
    bounds: Bounds,
    options: &Q2RereleaseOptions,
    force: bool,
) -> Option<ActorId> {
    let mut spots = q2_entities_named(game, "info_player_deathmatch");
    if spots.is_empty() {
        spots = q2_entities_named(game, "info_player_team1");
        spots.extend(q2_entities_named(game, "info_player_team2"));
    }
    if spots.is_empty() {
        if let Some(start) = q2_entities_named(game, "info_player_start").first() {
            spots = vec![start.clone()];
        }
    }
    if spots.is_empty() {
        panic!("Q2 rerelease: no valid spawn points found");
    }
    if spots.len() == 1 {
        let spot = spots.first().cloned();
        return match spot {
            Some(spot) if force || spawn_clear(game, &spot, bounds) => Some(spot),
            _ => None,
        };
    }
    let mut ranges: Vec<(ActorId, f64)> = spots
        .iter()
        .map(|spot| (spot.clone(), q2_players_range(game, spot.clone())))
        .collect();
    ranges.sort_by(|left, right| left.1.partial_cmp(&right.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut sorted: Vec<ActorId> = ranges.into_iter().map(|(spot, _)| spot).collect();
    if options.deathmatch_spawn_farthest {
        for spot in sorted.iter().rev() {
            if spawn_clear(game, spot, bounds) {
                return Some(spot.clone());
            }
        }
    } else {
        let mut index = sorted.len() - 1;
        while index > 2 {
            let destination = 2 + (game.random() * (index - 1) as f64).floor() as usize;
            sorted.swap(index, destination);
            index -= 1;
        }
        let mut order: Vec<ActorId> = sorted.iter().skip(2).cloned().collect();
        order.extend(sorted.iter().take(2).rev().cloned());
        for spot in &order {
            if spawn_clear(game, spot, bounds) {
                return Some(spot.clone());
            }
        }
    }
    if force {
        let pick = (game.random() * spots.len() as f64).floor() as usize;
        spots.get(pick).cloned()
    } else {
        None
    }
}

/// Select a coop spawn.
fn coop_spawn(
    game: &mut Q2GameServices,
    player: &ActorId,
    bounds: Bounds,
    options: &Q2RereleaseOptions,
    spawn_point: &str,
    force: bool,
) -> Option<ActorId> {
    let first = q2_rerelease_single_spawn(game, spawn_point);
    let coop = q2_entities_named(game, "info_player_coop");
    let matching: Vec<ActorId> = coop
        .iter()
        .filter(|spot| game.require_entity(spot).targetname.to_lowercase() == spawn_point.to_lowercase())
        .cloned()
        .collect();
    let candidates: Vec<ActorId> = if matching.is_empty() {
        coop.into_iter()
            .filter(|spot| game.require_entity(spot).targetname.is_empty())
            .collect()
    } else {
        matching
    };
    for check_players in [true, false] {
        let order: Vec<ActorId> = match &first {
            None => candidates.clone(),
            Some(first) => std::iter::once(first.clone())
                .chain(candidates.iter().cloned())
                .collect(),
        };
        for spot in &order {
            let mut origin = game.body_of(spot.clone()).origin;
            let exclude = if check_players {
                vec![player.clone()]
            } else {
                game.host.players()
            };
            let mask = if check_players { 0x42010003 } else { 0x2010003 };
            let trace_at = |game: &mut Q2GameServices, origin: Vec3| {
                game.host.trace(&Q2TraceRequest {
                    start: origin,
                    end: origin,
                    bounds: Some(bounds),
                    ignore: Some(player.clone()),
                    mask,
                    exclude: exclude.clone(),
                })
            };
            let mut result = trace_at(game, origin);
            if spawn_blocked(game, &result) {
                origin = add3(origin, vec3(0.0, 0.0, 1.0));
                result = trace_at(game, origin);
            }
            if spawn_blocked(game, &result) {
                let Some(fixed) = fix_q2_stuck_player(player.clone(), game, origin, bounds) else {
                    continue;
                };
                origin = fixed;
                result = trace_at(game, origin);
            }
            if result.fraction == 1.0
                || (!check_players && matches!(&result.hit, TraceHit::Actor { actor } if game.host.is_player(actor)))
            {
                return Some(spot.clone());
            }
        }
    }
    if force || !options.coop_player_collision {
        first
    } else {
        None
    }
}

/// Whether a spawn trace is blocked by the world (not a player).
fn spawn_blocked(game: &mut Q2GameServices, result: &crate::q2::support::contracts::TraceResult) -> bool {
    result.start_solid && !matches!(&result.hit, TraceHit::Actor { actor } if game.host.is_player(actor))
}

/// Select a rerelease spawn (`selectQ2RereleaseSpawn`).
pub fn select_q2_rerelease_spawn(
    game: &mut Q2GameServices,
    player: ActorId,
    bounds: Bounds,
    options: &Q2RereleaseOptions,
    spawn_point: &str,
    force: bool,
) -> Option<ActorId> {
    if game.options.mode == Q2Mode::Singleplayer {
        return q2_rerelease_single_spawn(game, spawn_point);
    }
    if game.options.mode == Q2Mode::Deathmatch {
        return deathmatch_spawn(game, bounds, options, force);
    }
    if game.options.map_name.to_lowercase() == "rmine2" {
        return lava_spawn(game);
    }
    coop_spawn(game, &player, bounds, options, spawn_point, force)
}
