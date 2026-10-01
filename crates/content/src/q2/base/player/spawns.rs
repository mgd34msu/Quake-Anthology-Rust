//! Q2 player spawns (`src/content/q2/base/player/spawns.ts`).

use std::collections::BTreeMap;

use qa_core::identity::ActorId;
use qa_core::math::{Vec3, add3, length3, sub3, vec3};

use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::host::{Q2GameServices, Q2Mode, Q2Solid, Q2TraceRequest};
use crate::q2::support::contracts::TraceHit;

use super::types::Q2PlayerState;

/// Coop maps with fixed spawn points.
const FIXED_COOP_MAPS: &[&str] = &[
    "jail2", "jail4", "mine1", "mine2", "mine3", "mine4", "lab", "boss1", "fact3", "biggun",
    "space", "command", "power2", "strike",
];

/// Entities with a classname in source order (`q2EntitiesNamed`).
pub fn q2_entities_named(game: &mut Q2GameServices, classname: &str) -> Vec<ActorId> {
    let mut entities: Vec<ActorId> = game
        .entities
        .values()
        .filter(|entity| entity.classname == classname)
        .map(|entity| entity.actor.id().clone())
        .collect();
    entities.sort_by_key(|actor| {
        game.host
            .actors()
            .source_of(actor)
            .map_or(actor.slot(), |(_, slot)| slot)
    });
    entities
}

/// Security coop spots fixup think.
fn security_coop_spots(actor: ActorId, game: &mut Q2GameServices) {
    let _ = actor;
    for x in [124.0, 252.0, 316.0] {
        let spot = game.create("info_player_coop", BTreeMap::new());
        game.require_entity_mut(&spot).targetname = "jail3".to_string();
        let mut moved = game.body_of(spot.clone());
        moved.origin = vec3(x, -164.0, 80.0);
        moved.angles = vec3(0.0, 90.0, 0.0);
        game.write_body(spot, &moved, true);
    }
}

/// Coop targetname fixup think.
fn coop_targetname_fix(actor: ActorId, game: &mut Q2GameServices) {
    for start in q2_entities_named(game, "info_player_start") {
        let targetname = game.require_entity(&start).targetname.clone();
        if targetname.is_empty() {
            continue;
        }
        let distance = length3(sub3(
            game.body_of(start.clone()).origin,
            game.body_of(actor.clone()).origin,
        ));
        if distance < 384.0 {
            let own = game.require_entity(&actor).targetname.clone();
            if own.to_lowercase() != targetname.to_lowercase() {
                game.require_entity_mut(&actor).targetname = targetname;
            }
            break;
        }
    }
}

/// Player spawn callbacks.
pub fn spawn_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks.think.insert("security_coop_spots", security_coop_spots as _);
    callbacks.think.insert("coop_targetname_fix", coop_targetname_fix as _);
    callbacks
}

/// Spawn a player spawn entity (`q2PlayerSpawns[spawn]`).
pub fn spawn_player_spawn(actor: ActorId, game: &mut Q2GameServices) -> bool {
    let classname = game.require_entity(&actor).classname.clone();
    match classname.as_str() {
        "info_player_start" => {
            if game.options.mode == Q2Mode::Coop
                && game.options.map_name.to_lowercase() == "security"
            {
                game.schedule(actor, 0.1, security_coop_spots as _);
            }
            true
        }
        "info_player_deathmatch" => {
            if game.options.mode != Q2Mode::Deathmatch {
                game.remove_actor(actor);
                return true;
            }
            {
                let entity = game.require_entity_mut(&actor);
                entity.model = "models/objects/dmspot/tris.md2".to_string();
                entity.skin = 1;
            }
            let mut moved = game.body_of(actor.clone());
            moved.bounds.min = vec3(-32.0, -32.0, -24.0);
            moved.bounds.max = vec3(32.0, 32.0, -16.0);
            game.write_body(actor.clone(), &moved, false);
            game.set_solid(actor.clone(), Q2Solid::Box);
            game.show(actor);
            true
        }
        "info_player_coop" => {
            if game.options.mode != Q2Mode::Coop {
                game.remove_actor(actor);
                return true;
            }
            if FIXED_COOP_MAPS.contains(&game.options.map_name.to_lowercase().as_str()) {
                game.schedule(actor, 0.1, coop_targetname_fix as _);
            }
            true
        }
        "info_player_intermission" => true,
        _ => false,
    }
}

/// Closest live player distance (`q2PlayersRange`).
pub fn q2_players_range(game: &mut Q2GameServices, spot: ActorId) -> f64 {
    let mut closest: f64 = 9999999.0;
    let origin = game.body_of(spot).origin;
    for player in game.host.players() {
        let health = game
            .host
            .combat()
            .read(&player)
            .map_or(0.0, |combat| combat.health);
        if health <= 0.0 {
            continue;
        }
        if let Some(body) = game.host.bodies().read(&player) {
            closest = closest.min(f64::from(length3(sub3(body.origin, origin))));
        }
    }
    closest
}

/// Select a spawn (`selectQ2Spawn`).
pub fn select_q2_spawn(
    game: &mut Q2GameServices,
    state: &Q2PlayerState,
    spawn_point: &str,
) -> ActorId {
    let mut spot: Option<ActorId> = None;
    if game.options.mode == Q2Mode::Deathmatch {
        let spots = q2_entities_named(game, "info_player_deathmatch");
        if game.options.deathmatch_flags & 512 != 0 {
            let mut best = 0.0;
            for candidate in &spots {
                let distance = q2_players_range(game, candidate.clone());
                if distance > best {
                    best = distance;
                    spot = Some(candidate.clone());
                }
            }
            if spot.is_none() {
                spot = spots.first().cloned();
            }
        } else {
            let mut first: Option<ActorId> = None;
            let mut second: Option<ActorId> = None;
            let mut first_range = 99999.0;
            let mut second_range = 99999.0;
            for candidate in &spots {
                let range = q2_players_range(game, candidate.clone());
                // Preserve source selection, including its non-shifting first/second closest slots.
                if range < first_range {
                    first_range = range;
                    first = Some(candidate.clone());
                } else if range < second_range {
                    second_range = range;
                    second = Some(candidate.clone());
                }
            }
            let count = if spots.len() <= 2 {
                spots.len() as i32
            } else {
                spots.len() as i32 - 2
            };
            if count > 0 {
                let mut selection = (game.host.random() * f64::from(count)).floor() as i32;
                for candidate in &spots {
                    if spots.len() > 2
                        && (Some(candidate) == first.as_ref()
                            || Some(candidate) == second.as_ref())
                    {
                        selection += 1;
                    }
                    if selection == 0 {
                        spot = Some(candidate.clone());
                        break;
                    }
                    selection -= 1;
                }
            }
        }
    } else if game.options.mode == Q2Mode::Coop && state.slot != 0 {
        spot = q2_entities_named(game, "info_player_coop")
            .into_iter()
            .filter(|candidate| {
                game.require_entity(candidate).targetname.to_lowercase()
                    == spawn_point.to_lowercase()
            })
            .nth((state.slot - 1) as usize);
    }
    if spot.is_none() {
        let starts = q2_entities_named(game, "info_player_start");
        spot = starts.iter().find(|candidate| {
            let targetname = game.require_entity(candidate).targetname.clone();
            if spawn_point.is_empty() {
                targetname.is_empty()
            } else {
                targetname.to_lowercase() == spawn_point.to_lowercase()
            }
        }).cloned();
        if spot.is_none() && spawn_point.is_empty() {
            spot = starts.first().cloned();
        }
    }
    spot.unwrap_or_else(|| {
        panic!(
            "No Q2 player spawn for '{spawn_point}' on {}",
            game.options.map_name
        )
    })
}

/// Kill everything at an entity's position (`q2KillBox`).
pub fn q2_kill_box(actor: ActorId, game: &mut Q2GameServices) -> bool {
    let zero = vec3(0.0, 0.0, 0.0);
    let body = game.body_of(actor.clone());
    loop {
        let trace = game.host.trace(&Q2TraceRequest {
            start: body.origin,
            end: body.origin,
            bounds: Some(body.bounds),
            ignore: Some(actor.clone()),
            mask: 0x2010003,
            exclude: Vec::new(),
        });
        match &trace.hit {
            TraceHit::Actor { actor: target } => {
                let target = target.clone();
                game.damage(
                    target.clone(),
                    actor.clone(),
                    Some(actor.clone()),
                    100000.0,
                    0.0,
                    zero,
                    body.origin,
                    zero,
                    21,
                    32,
                    None,
                );
                if game.host.actors().is_live(&target) {
                    match game.entity(&target) {
                        None => return false,
                        Some(other) => {
                            if other.solid != Q2Solid::None {
                                return false;
                            }
                        }
                    }
                }
            }
            _ => return !trace.start_solid,
        }
    }
}

/// Spawned player origin (`q2SpawnOrigin`).
pub fn q2_spawn_origin(game: &mut Q2GameServices, actor: ActorId) -> Vec3 {
    add3(game.body_of(actor).origin, vec3(0.0, 0.0, 10.0))
}
