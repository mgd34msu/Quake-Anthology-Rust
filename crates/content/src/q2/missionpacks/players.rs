//! Rogue player spawns (`src/content/q2/missionpacks/players.ts`).
//!
//! Original Rogue p_client.c lava-level cooperative spawn selection.

use qa_core::identity::ActorId;
use qa_core::math::{add3, length3, scale3, sub3, Vec3};

use crate::q2::foundation::host::{Q2Edition, Q2GameServices, Q2Mode, Q2SpawnFn, SpawnModule};

/// Rogue player spawns (`Q2RoguePlayerSpawns`).
#[derive(Debug, Clone, Copy)]
pub struct Q2RoguePlayerSpawns;

impl Q2RoguePlayerSpawns {
    /// Spawn a cooperative lava spawn point (`spawn`).
    pub fn spawn(&self, entity: ActorId, game: &mut Q2GameServices) -> bool {
        if game.options.edition != Q2Edition::Classic
            || game.require_entity(&entity).classname != "info_player_coop_lava"
        {
            return false;
        }
        if game.options.mode != Q2Mode::Coop {
            game.remove_actor(entity);
        }
        true
    }

    /// Select a lava spawn point (`selectSpawn`).
    pub fn select_spawn(&self, game: &mut Q2GameServices) -> Option<(Vec3, Vec3)> {
        if game.options.edition != Q2Edition::Classic
            || game.options.mode != Q2Mode::Coop
            || !["rmine2", "rmine2p"].contains(&game.options.map_name.to_lowercase().as_str())
        {
            return None;
        }
        let mut lava_top = -99999.0f32;
        let mut highest_lava: Option<ActorId> = None;
        let doors: Vec<ActorId> = game.entities.values().map(|entity| entity.actor.id().clone()).collect();
        for lava in doors {
            let record = game.require_entity(&lava);
            if record.classname != "func_door" || record.spawnflags & 2 == 0 {
                continue;
            }
            let body = game.body_of(lava.clone());
            let center = add3(body.origin, scale3(add3(body.bounds.min, body.bounds.max), 0.5));
            if game.host.point_contents(center) & 56 != 0 && body.origin.z + body.bounds.max.z > lava_top {
                lava_top = body.origin.z + body.bounds.max.z;
                highest_lava = Some(lava);
            }
        }
        highest_lava.as_ref()?;
        lava_top += 64.0;
        let mut count = 0;
        let mut lowest = 999999.0f32;
        let mut selected: Option<ActorId> = None;
        let points: Vec<ActorId> = game.entities.values().map(|entity| entity.actor.id().clone()).collect();
        for point in points {
            if game.require_entity(&point).classname != "info_player_coop_lava" {
                continue;
            }
            count += 1;
            if count > 64 {
                break;
            }
            let origin = game.body_of(point.clone()).origin;
            if origin.z < lava_top {
                continue;
            }
            let mut distance: f64 = 9999999.0;
            for player in game.host.players() {
                let body = game.host.bodies().read(&player);
                if let Some(body) = body {
                    if game
                        .host
                        .combat()
                        .read(&player)
                        .map(|combat| combat.health)
                        .unwrap_or(0.0)
                        > 0.0
                    {
                        distance = distance.min(f64::from(length3(sub3(origin, body.origin))));
                    }
                }
            }
            if distance > 32.0 && origin.z < lowest {
                selected = Some(point);
                lowest = origin.z;
            }
        }
        let selected = selected?;
        let body = game.body_of(selected);
        Some((add3(body.origin, Vec3 { x: 0.0, y: 0.0, z: 9.0 }), body.angles))
    }
}

/// Spawn entry for the module table.
fn rogue_spawn(entity: ActorId, game: &mut Q2GameServices) -> bool {
    Q2RoguePlayerSpawns.spawn(entity, game)
}

/// Spawn module entry (`Q2SpawnModule`).
pub fn rogue_player_spawn_module() -> SpawnModule {
    SpawnModule {
        spawn: rogue_spawn as Q2SpawnFn,
        item_name: |_| None,
        callbacks: crate::q2::foundation::callbacks::Q2CallbackDefinitions::default(),
    }
}
