//! Q1 mg3 monster startup (`src/content/q1/addons/monsters/startup.ts`).
//!
//! `quakec_{mg1,mg3}/monsters.qc` InitMonster, StartMonster,
//! monster_begin_walking. GPL-2.0-or-later.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

use crate::q1::addons::context::{
    addon_cvar, removed_for_runes, removed_outside_coop, set_addon_number, set_addon_vector, set_combat_team,
    Q1AddonContext, Q1AddonProgram,
};
use crate::q1::foundation::callbacks::{callback_name, Q1CallbackHandlers, Q1UseHandler};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::BodyPatch;
use crate::q1::foundation::types::{vadd, vsub, yaw_for, Q1Effect, Q1MoveType, Q1Solid, Q1TraceRequest};

use super::ai::Mg3Monster;
use crate::q1::{q1_error, Q1Error};

const SMALL: Bounds = Bounds {
    min: Vec3 {
        x: -16.0,
        y: -16.0,
        z: -24.0,
    },
    max: Vec3 {
        x: 16.0,
        y: 16.0,
        z: 40.0,
    },
};

const LARGE: Bounds = Bounds {
    min: Vec3 {
        x: -32.0,
        y: -32.0,
        z: -24.0,
    },
    max: Vec3 {
        x: 32.0,
        y: 32.0,
        z: 64.0,
    },
};

fn start_names() -> &'static Mutex<HashMap<usize, HashSet<String>>> {
    static NAMES: OnceLock<Mutex<HashMap<usize, HashSet<String>>>> = OnceLock::new();
    NAMES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn startup_key(game: &Q1EntityServices) -> usize {
    std::ptr::from_ref(game) as usize
}

/// Initialize an mg3 monster. Native spawn functions set their own
/// health and callbacks before InitMonster (`initMg3Monster`).
pub fn init_mg3_monster(
    monster: &mut Mg3Monster,
    context: &Q1AddonContext,
    model: &str,
    kind: u32,
    size: u32,
) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    if monster.monster.game.options().deathmatch != 0
        || removed_for_runes(monster.monster.game, &id)?
        || removed_outside_coop(monster.monster.game, &id, false)?
    {
        if monster.monster.game.is_live(&id) {
            monster.monster.game.remove(&id)?;
        }
        return Ok(());
    }
    monster.monster.game.update_entity(&id, |entity| {
        entity.movement_flags |= 16384;
        entity.fields.insert(String::from("mdl"), model.to_string());
    })?;
    set_addon_number(monster.monster.game, &id, "lefty", f64::from(kind))?;
    set_addon_number(monster.monster.game, &id, "state", f64::from(size))?;
    monster.monster.controller.lefty = true;
    if let Some(mission) = monster.monster.game.monster_missions.get_mut(&id) {
        mission.spawned();
    } else {
        let spawnflags = monster
            .monster
            .game
            .entity_ref(&id)
            .map(|entity| entity.spawnflags)
            .unwrap_or(0);
        if context.program() == Q1AddonProgram::Mg3
            || addon_cvar(monster.monster.game, "horde")? == 0.0
            || spawnflags & 4 == 0
        {
            monster.monster.game.total_monsters += 1;
        }
    }
    if context.program() == Q1AddonProgram::Mg3
        && monster
            .monster
            .game
            .entity_ref(&id)
            .map(|entity| entity.text("health_target"))
            .unwrap_or_default()
            != ""
    {
        let health = monster.monster.game.health(&id);
        monster.monster.game.update_entity(&id, |entity| {
            entity.max_health = health;
        })?;
    }
    let entity = monster
        .monster
        .game
        .entity_ref(&id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    if entity.spawnflags & 4 != 0 {
        let prefix = monster.monster.prefix.clone();
        let use_callback = monster.monster.game.named.use_callback(&format!("{prefix}:start"))?;
        monster.monster.game.update_entity(&id, |entity| {
            entity.use_callback = Some(use_callback);
        })?;
        return Ok(());
    }
    let delay = entity.next_think.max(0.0) + monster.monster.game.host.random() * 0.5 - monster.monster.game.time;
    let prefix = monster.monster.prefix.clone();
    let start = monster.monster.game.named.action(&format!("{prefix}:monster_start"))?;
    monster.monster.game.schedule(&id, delay, &start)
}

/// Start an mg3 monster after map spawn (`startMg3Monster`).
pub fn start_mg3_monster(monster: &mut Mg3Monster, context: &Q1AddonContext) -> Result<(), Q1Error> {
    let id = monster.monster.id.clone();
    let entity = monster
        .monster
        .game
        .entity_ref(&id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let bounds = if entity.number("state") == 1.0 { SMALL } else { LARGE };
    let model = entity.text("mdl");
    monster.monster.game.update_entity(&id, |entity| {
        entity.solid = Q1Solid::Slidebox;
        entity.movement = Q1MoveType::Step;
        entity.model = model;
    })?;
    monster.monster.game.set_bounds(&id, bounds)?;
    if entity.number("lefty") == 1.0 {
        let origin = monster.monster.origin()?;
        let start = vadd(origin, Vec3 { x: 0.0, y: 0.0, z: 1.0 });
        let trace = monster.monster.game.host.trace(&Q1TraceRequest {
            start,
            end: vsub(
                start,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 256.0,
                },
            ),
            bounds,
            ignore: Some(id.clone()),
            monsters: true,
            missile: false,
        });
        if trace.fraction < 1.0 && !trace.all_solid {
            let ground = trace.actor.clone();
            monster.monster.game.set_body(
                &id,
                &BodyPatch {
                    origin: Some(trace.end),
                    ground: Some(ground),
                    ..Default::default()
                },
            )?;
            monster.monster.game.update_entity(&id, |entity| {
                entity.movement_flags |= 512;
            })?;
        } else {
            monster.monster.game.set_body(
                &id,
                &BodyPatch {
                    origin: Some(start),
                    ..Default::default()
                },
            )?;
        }
    }
    let owned = monster
        .monster
        .game
        .entity_ref(&id)
        .map(|entity| entity.actor.clone())
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    monster.monster.game.host.walk_move(&owned, 0.0, 0.0);
    let entity = monster
        .monster
        .game
        .entity_ref(&id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    if entity.spawnflags & 4 != 0 {
        let death = monster.monster.game.create("teledeath", None, None)?;
        let touch = monster.monster.game.named.touch("tdeath_touch")?;
        monster.monster.game.update_entity(&death, |entity| {
            entity.owner = Some(id.clone());
            entity.solid = Q1Solid::Trigger;
            entity.touch = Some(touch);
        })?;
        let origin = monster.monster.origin()?;
        monster.monster.game.set_body(
            &death,
            &BodyPatch {
                origin: Some(origin),
                bounds: Some(Bounds {
                    min: vsub(bounds.min, Vec3 { x: 1.0, y: 1.0, z: 1.0 }),
                    max: vadd(bounds.max, Vec3 { x: 1.0, y: 1.0, z: 1.0 }),
                }),
                ..Default::default()
            },
        )?;
        monster.monster.game.link(&death)?;
        let remove = monster.monster.game.named.action("SUB_Remove")?;
        monster.monster.game.schedule(&death, 0.2, &remove)?;
        if entity.spawnflags & 16 != 0 {
            monster.monster.game.effect(Q1Effect::Teleport, origin, None, 1);
        }
    }
    let yaw = monster.monster.game.body(&id).map(|body| body.angles.y)?;
    monster.monster.game.update_entity(&id, |entity| {
        entity.aimed_damage = true;
        entity.ideal_yaw = f64::from(yaw);
        if entity.yaw_speed == 0.0 {
            entity.yaw_speed = 20.0;
        }
    })?;
    monster.monster.game.set_damageable(&id, true)?;
    set_addon_vector(
        monster.monster.game,
        &id,
        "view_ofs",
        Vec3 {
            x: 0.0,
            y: 0.0,
            z: 25.0,
        },
    )?;
    let prefix = monster.monster.prefix.clone();
    let use_callback = monster
        .monster
        .game
        .named
        .use_callback(&format!("{prefix}:monster_use"))?;
    let kind = monster
        .monster
        .game
        .entity_ref(&id)
        .map(|entity| entity.number("lefty"))
        .unwrap_or(0.0);
    monster.monster.game.update_entity(&id, |entity| {
        entity.use_callback = Some(use_callback);
        entity.movement_flags |= 32
            | if kind == 2.0 {
                1
            } else if kind == 3.0 {
                2
            } else {
                0
            };
    })?;
    let owned = monster
        .monster
        .game
        .entity_ref(&id)
        .map(|entity| entity.actor.clone())
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    set_combat_team(monster.monster.game, &owned, Some("q1:monsters"))?;
    monster.monster.game.link(&id)?;
    if monster.monster.game.monster_missions.contains_key(&id) {
        if let Some(mission) = monster.monster.game.monster_missions.get_mut(&id) {
            mission.started();
        }
        monster.update_route()?;
        let entity = monster
            .monster
            .game
            .entity_ref(&id)
            .cloned()
            .ok_or_else(|| q1_error("Missing Q1 entity"))?;
        let delay = entity.next_think - monster.monster.game.time + monster.monster.game.host.random() * 0.5;
        return monster.monster.delay(delay);
    }
    let entity = monster
        .monster
        .game
        .entity_ref(&id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let targets = if entity.target.is_empty() {
        Vec::new()
    } else {
        monster.monster.game.find(&entity.target.clone())
    };
    if entity.spawnflags & 32 != 0 && !entity.target.is_empty() {
        let alive: Vec<ActorId> = targets
            .iter()
            .filter(|target| monster.monster.game.is_damageable(target))
            .cloned()
            .collect();
        if !alive.is_empty() {
            let index =
                ((alive.len() as f64 * monster.monster.game.host.random()).floor() as usize).min(alive.len() - 1);
            let chosen = alive[index].clone();
            return monster.found(&chosen);
        }
    }
    let target = targets.first().cloned();
    let target_classname = target.as_ref().and_then(|target| {
        monster
            .monster
            .game
            .entity_ref(target)
            .map(|entity| entity.classname.clone())
    });
    monster.monster.game.update_entity(&id, |entity| {
        entity.references.insert(String::from("movetarget"), target.clone());
        entity.references.insert(String::from("goalentity"), target.clone());
    })?;
    monster.monster.monster.path = if target.is_some() {
        entity.target.clone()
    } else {
        String::new()
    };
    if let Some(target) = target.as_ref() {
        let end = monster.monster.game.body(target).map(|body| body.origin)?;
        let origin = monster.monster.origin()?;
        monster.monster.game.update_entity(&id, |entity| {
            entity.ideal_yaw = yaw_for(vsub(end, origin));
        })?;
    }
    if target_classname.as_deref() == Some("path_corner") && entity.spawnflags & 4096 == 0 {
        let walk = monster.monster.spec.walk;
        monster.play(walk)?;
    } else {
        monster.monster.monster.pause_until = 99999999.0;
        let stand = monster.monster.spec.stand;
        monster.play(stand)?;
        if target_classname.as_deref() == Some("path_corner") {
            let prefix = monster.monster.prefix.clone();
            let walk = monster.monster.game.named.use_callback(&format!("{prefix}:walk"))?;
            monster.monster.game.update_entity(&id, |entity| {
                entity.use_callback = Some(walk);
            })?;
        }
    }
    if entity.spawnflags & 4 == 0 {
        let delay = entity.next_think - monster.monster.game.time + monster.monster.game.host.random() * 0.5;
        return monster.monster.delay(delay);
    }
    if context.program() != Q1AddonProgram::Mg3
        && monster.monster.game.entity_ids().iter().any(|id| {
            monster
                .monster
                .game
                .entity_ref(id)
                .is_some_and(|entity| entity.classname == "horde_manager")
        })
    {
        monster.monster.game.total_monsters += 1;
    }
    if entity.spawnflags & 8 != 0 {
        let activator = entity.activator.clone();
        monster.use_monster(activator.as_ref())?;
    }
    Ok(())
}

/// Resolve the player an mg3 monster use applies to
/// (`mg3MonsterActivator`).
pub fn mg3_monster_activator(game: &mut Q1EntityServices, activator: Option<&ActorId>) -> Option<ActorId> {
    if activator.is_some_and(|activator| game.is_player(activator)) {
        return activator.cloned();
    }
    (game.host.players)()
        .into_iter()
        .find(|player| game.health(player) > 0.0)
}

fn mg3_start_use_handler(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let activator = activator.cloned();
    game.update_entity(id, |entity| {
        entity.activator = activator;
    })?;
    let mut monster = Mg3Monster::load(game, id)?;
    monster.start()?;
    monster.finish()
}

fn mg3_walk_use_handler(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let prefix = game
        .entity_ref(id)
        .map(|entity| entity.text("source.monsterCallbackPrefix"))
        .unwrap_or_default();
    let mut monster = Mg3Monster::load(game, id)?;
    if monster.monster.game.health(&monster.monster.id.clone()) <= 0.0 || monster.monster.monster.enemy.is_some() {
        let id = monster.monster.id.clone();
        monster.monster.game.update_entity(&id, |entity| {
            entity.use_callback = None;
        })?;
        return monster.finish();
    }
    let use_callback = monster
        .monster
        .game
        .named
        .use_callback(&format!("{prefix}:monster_use"))?;
    let id = monster.monster.id.clone();
    monster.monster.game.update_entity(&id, |entity| {
        entity.use_callback = Some(use_callback);
    })?;
    monster.monster.monster.pause_until = 0.0;
    let walk = monster.monster.spec.walk;
    monster.play(walk)?;
    monster.finish()
}

/// Register mg3 monster start and walk use-handlers
/// (`registerMg3MonsterStartup`). Handlers load the mg3 controller for
/// the used entity, which replaces the donor loader closure.
pub fn register_mg3_monster_startup(game: &mut Q1EntityServices, name: &str) -> Result<(), Q1Error> {
    start_names()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entry(startup_key(game))
        .or_default()
        .insert(format!("{name}:start"));
    game.named.register(
        &format!("{name}:start"),
        Q1CallbackHandlers {
            use_callback: Some(mg3_start_use_handler as Q1UseHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{name}:walk"),
        Q1CallbackHandlers {
            use_callback: Some(mg3_walk_use_handler as Q1UseHandler),
            ..Default::default()
        },
    )?;
    Ok(())
}

/// Whether an entity still waits on its mg3 start use-handler
/// (`waitingMg3Monster`).
pub fn waiting_mg3_monster(game: &Q1EntityServices, id: &ActorId) -> bool {
    let name = game
        .entity_ref(id)
        .and_then(|entity| callback_name(entity.use_callback.as_ref()));
    name.as_ref().is_some_and(|name| {
        start_names()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&startup_key(game))
            .is_some_and(|names| names.contains(name))
    })
}
