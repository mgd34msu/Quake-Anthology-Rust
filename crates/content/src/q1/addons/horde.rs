//! Q1 horde addon (`src/content/q1/addons/horde/index.ts`).
//!
//! `quakec_mg1/horde.qc`, `combat.qc` and `client.qc` wave manager.
//! GPL-2.0-or-later.

pub mod loot;
pub mod squads;
pub mod types;

pub use crate::q1::addons::horde::types::Q1HordeServices;

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

use crate::q1::addons::context::{
    addon_alpha, addon_broadcast, addon_cvar, addon_frame_time, addon_player_number, addon_program, addon_set_cvar,
    fround, set_addon_number, set_addon_player_number, set_addon_vector, Q1AddonProgram,
};
use crate::q1::addons::horde::loot::{register_horde_loot, spawn_horde_powerup};
use crate::q1::addons::horde::squads::{choose_horde_squad, horde_squad, HordeCategory, HordeMonster, HordeSquadType};
use crate::q1::addons::monsters::ordinary::ordinary_addon_monster;
use crate::q1::base::monsters::BaseMonster;
use crate::q1::base::provider::{register_kill_count_rule, update_base};
use crate::q1::foundation::callbacks::{
    callback_name, Q1ActionHandler, Q1CallbackHandlers, Q1DieHandler, Q1TouchHandler, Q1UseHandler,
};
use crate::q1::foundation::entity::Q1MonsterSpecies;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::extensions::{Q1PlayerExtension, Q1WeaponRules};
use crate::q1::foundation::gameplay::{BodyPatch, CombatTraits, TouchSurface};
use crate::q1::foundation::movers::spawn_door;
use crate::q1::foundation::types::{
    overlaps, vadd, vsub, Q1Effect, Q1MessageArg, Q1MoveType, Q1Solid, Q1SoundChannel, Q1TraceRequest, Q1Weapon,
};
use crate::q1::{q1_error, Q1Error};

/// Per-game horde registration.
struct HordeRegistration {
    /// Session services.
    services: Box<dyn Q1HordeServices>,
}

fn registry() -> &'static Mutex<HashMap<usize, HordeRegistration>> {
    static STATES: OnceLock<Mutex<HashMap<usize, HordeRegistration>>> = OnceLock::new();
    STATES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock_registry() -> std::sync::MutexGuard<'static, HashMap<usize, HordeRegistration>> {
    registry().lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Registry key for a game.
fn horde_key(game: &Q1EntityServices) -> usize {
    std::ptr::from_ref(game) as usize
}

/// Register per-game horde state, overwriting any previous entry.
fn register_horde_state(game: &mut Q1EntityServices, services: Box<dyn Q1HordeServices>) {
    lock_registry().insert(horde_key(game), HordeRegistration { services });
}

/// Run a session services operation. The closure receives only the
/// services: game calls cannot run while the registry is locked, which
/// rules out reentrant deadlocks by construction.
fn with_horde_services<T>(
    game: &Q1EntityServices,
    op: impl FnOnce(&mut dyn Q1HordeServices) -> T,
) -> Result<T, Q1Error> {
    let mut states = lock_registry();
    let state = states
        .get_mut(&horde_key(game))
        .ok_or_else(|| q1_error("Q1 horde was not registered"))?;
    Ok(op(state.services.as_mut()))
}

fn number(game: &Q1EntityServices, id: &ActorId, name: &str) -> f64 {
    game.entity_ref(id).map(|entity| entity.number(name)).unwrap_or(0.0)
}

/// Horde monster classname (`spawnMonster`).
fn horde_monster_classname(kind: HordeMonster) -> &'static str {
    match kind {
        HordeMonster::Grunt => "monster_army",
        HordeMonster::Hellknight => "monster_hell_knight",
        HordeMonster::Demon => "monster_demon1",
        HordeMonster::Knight => "monster_knight",
        HordeMonster::Dog => "monster_dog",
        HordeMonster::Ogre => "monster_ogre",
        HordeMonster::Enforcer => "monster_enforcer",
        HordeMonster::Shambler => "monster_shambler",
        HordeMonster::Shalrath => "monster_shalrath",
        HordeMonster::Wizard => "monster_wizard",
        HordeMonster::Zombie => "monster_zombie",
    }
}

/// Horde monster donor name (`HordeMonster`).
fn horde_monster_name(kind: HordeMonster) -> &'static str {
    match kind {
        HordeMonster::Knight => "knight",
        HordeMonster::Hellknight => "hellknight",
        HordeMonster::Dog => "dog",
        HordeMonster::Demon => "demon",
        HordeMonster::Ogre => "ogre",
        HordeMonster::Grunt => "grunt",
        HordeMonster::Enforcer => "enforcer",
        HordeMonster::Shambler => "shambler",
        HordeMonster::Shalrath => "shalrath",
        HordeMonster::Wizard => "wizard",
        HordeMonster::Zombie => "zombie",
    }
}

/// Horde spawn-point classname (`findSpawn`).
fn horde_spawn_classname(kind: HordeSquadType) -> &'static str {
    match kind {
        HordeSquadType::Normal => "info_monster_start",
        HordeSquadType::Ranged => "info_monster_start_ranged",
        HordeSquadType::Flying => "info_monster_start_flying",
        HordeSquadType::Boss => "info_monster_start_boss",
    }
}

/// Horde manager entity (`manager`).
pub(crate) fn horde_manager(game: &Q1EntityServices) -> Option<ActorId> {
    game.entity_ids().into_iter().find(|id| {
        game.entity_ref(id)
            .is_some_and(|entity| entity.classname == "horde_manager")
    })
}

/// Schedule a horde action (`schedule`).
pub(crate) fn horde_schedule(game: &mut Q1EntityServices, id: &ActorId, name: &str, delay: f64) -> Result<(), Q1Error> {
    game.schedule(id, delay, &format!("mg1:horde:{name}"))
}

/// Living players (`livingPlayers`).
pub(crate) fn horde_living_players(game: &mut Q1EntityServices) -> Vec<ActorId> {
    (game.host.players)()
        .into_iter()
        .filter(|player| game.health(player) > 0.0)
        .collect()
}

/// Living non-zombie horde monsters (`livingMonsters`).
fn horde_living_monsters(game: &Q1EntityServices) -> Vec<ActorId> {
    game.entity_ids()
        .into_iter()
        .filter(|id| {
            game.entity_ref(id)
                .is_some_and(|entity| entity.text("category") == "monster" && entity.classname != "monster_zombie")
                && game.health(id) > 0.0
        })
        .collect()
}

/// Whether a player is a bot (`services.isBot`).
pub(crate) fn horde_is_bot(game: &Q1EntityServices, player: &ActorId) -> Result<bool, Q1Error> {
    with_horde_services(game, |services| services.is_bot(player))
}

/// Pick a monster target (`target`).
fn horde_target(game: &mut Q1EntityServices) -> Result<Option<ActorId>, Q1Error> {
    let players = (game.host.players)();
    let count = players.iter().filter(|player| game.health(player) > 0.0).count();
    if count == 0 {
        return Ok(None);
    }
    let roll = game.host.random() * count as f64;
    let mut index = 0;
    for player in &players {
        if game.health(player) > 0.0 {
            index += 1;
        }
        if roll <= f64::from(index) && !with_horde_services(game, |services| services.no_target(player))? {
            return Ok(Some(player.clone()));
        }
    }
    Ok(None)
}

/// Whether a spawn point is blocked (`blocked`).
fn horde_blocked(game: &mut Q1EntityServices, point: &ActorId) -> Result<bool, Q1Error> {
    if game.entity_ref(point).map(|entity| entity.spawnflags).unwrap_or(0) & 2 != 0 {
        return Ok(false);
    }
    let source = game.body(point)?;
    let min = vadd(source.origin, source.bounds.min);
    let max = vadd(source.origin, source.bounds.max);
    for player in horde_living_players(game) {
        if with_horde_services(game, |services| services.dead_flag(&player))? > 0 {
            continue;
        }
        let Some(body) = game.host.bodies.read(&player) else {
            continue;
        };
        let low = vadd(body.origin, body.bounds.min);
        let high = vadd(body.origin, body.bounds.max);
        // Official CheckBlockedSpawn repeats the Y comparison and does not test Z.
        if high.x > min.x && low.x < max.x && high.y > min.y && low.y < max.y {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether a spawn point is usable (`findSpawn` validity).
fn horde_spawn_valid(game: &mut Q1EntityServices, point: &ActorId) -> Result<bool, Q1Error> {
    if game.time <= game.entity_ref(point).map(|entity| entity.wait).unwrap_or(0.0) {
        return Ok(false);
    }
    if game.entity_ref(point).map(|entity| entity.spawnflags).unwrap_or(0) & 1 != 0 {
        return Ok(false);
    }
    Ok(!horde_blocked(game, point)?)
}

/// Whether any candidate point is usable.
fn horde_spawn_any_valid(game: &mut Q1EntityServices, candidates: &[ActorId]) -> Result<bool, Q1Error> {
    for point in candidates {
        if horde_spawn_valid(game, point)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Find a spawn point (`findSpawn`).
fn horde_find_spawn(game: &mut Q1EntityServices, kind: HordeSquadType) -> Result<Option<ActorId>, Q1Error> {
    let points = |game: &Q1EntityServices, kind: HordeSquadType| {
        game.entity_ids()
            .into_iter()
            .filter(|id| {
                game.entity_ref(id)
                    .is_some_and(|entity| entity.classname == horde_spawn_classname(kind))
            })
            .collect::<Vec<_>>()
    };
    let mut candidates = points(game, kind);
    if !horde_spawn_any_valid(game, &candidates)? {
        if kind == HordeSquadType::Normal {
            return Ok(None);
        }
        candidates = points(game, HordeSquadType::Normal);
        if !horde_spawn_any_valid(game, &candidates)? {
            return Ok(None);
        }
    }
    let roll = candidates.len() as f64 * game.host.random();
    for (index, point) in candidates.iter().enumerate() {
        if index as f64 + 1.0 >= roll && horde_spawn_valid(game, point)? {
            return Ok(Some(point.clone()));
        }
    }
    for point in &candidates {
        if horde_spawn_valid(game, point)? {
            return Ok(Some(point.clone()));
        }
    }
    Ok(None)
}

/// Spawn a horde monster (`spawnMonster`).
fn horde_spawn_monster(
    game: &mut Q1EntityServices,
    kind: HordeMonster,
    origin: Vec3,
    angles: Vec3,
    owner: &ActorId,
) -> Result<ActorId, Q1Error> {
    let id = game.create(horde_monster_classname(kind), None, None)?;
    game.set_body(
        &id,
        &BodyPatch {
            origin: Some(origin),
            angles: Some(angles),
            ..Default::default()
        },
    )?;
    game.spawn_entity(&id, None)?;
    let native = game
        .entity_ref(&id)
        .and_then(|entity| entity.monster.clone())
        .is_some_and(|monster| matches!(monster.species, Q1MonsterSpecies::Army | Q1MonsterSpecies::Dog));
    let spec = match ordinary_addon_monster(game, &id)? {
        Some(controller) => {
            let spec = controller.spec;
            controller.finish()?;
            Some(spec)
        }
        None => None,
    };
    if spec.is_none() && !native {
        return Err(q1_error(format!(
            "Horde has no native monster controller for {}",
            horde_monster_name(kind)
        )));
    }
    game.cancel(&id);
    if let Some(spec) = spec {
        let actor = game
            .entity_ref(&id)
            .map(|entity| entity.actor.clone())
            .ok_or_else(|| q1_error("Missing Q1 horde monster"))?;
        game.update_entity(&id, |entity| {
            entity.model = format!("progs/{}.mdl", spec.model);
            entity.solid = Q1Solid::Slidebox;
            entity.movement = Q1MoveType::Step;
            entity.aimed_damage = true;
        })?;
        game.set_bounds(&id, spec.bounds)?;
        set_addon_vector(
            game,
            &id,
            "view_ofs",
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 25.0,
            },
        )?;
        if let Some(combat) = game.host.combat.read(actor.id()) {
            game.host.combat.set_traits(
                &actor,
                CombatTraits {
                    can_take_damage: combat.can_take_damage,
                    mass: combat.mass,
                    invulnerable: combat.invulnerable,
                    team: Some(String::from("q1:monsters")),
                    no_knockback: combat.no_knockback,
                },
            )?;
        }
        if kind == HordeMonster::Wizard {
            game.update_entity(&id, |entity| entity.movement_flags |= 1)?;
        }
    }
    let offset = match kind {
        HordeMonster::Demon => 48.0,
        HordeMonster::Ogre
        | HordeMonster::Shambler
        | HordeMonster::Shalrath
        | HordeMonster::Wizard
        | HordeMonster::Zombie => 32.0,
        _ => 24.0,
    };
    let mut position = vadd(
        origin,
        Vec3 {
            x: 0.0,
            y: 0.0,
            z: offset + 1.0,
        },
    );
    if kind != HordeMonster::Wizard {
        let bounds = game.body(&id)?.bounds;
        let actor = game
            .entity_ref(&id)
            .map(|entity| entity.actor.id().clone())
            .ok_or_else(|| q1_error("Missing Q1 horde monster"))?;
        let trace = game.host.trace(&Q1TraceRequest {
            start: position,
            end: vsub(
                position,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 256.0,
                },
            ),
            bounds,
            ignore: Some(actor),
            monsters: true,
            missile: false,
        });
        if trace.fraction < 1.0 && !trace.all_solid {
            position = trace.end;
            game.set_body(
                &id,
                &BodyPatch {
                    ground: Some(trace.actor.clone()),
                    ..Default::default()
                },
            )?;
            game.update_entity(&id, |entity| entity.movement_flags |= 512)?;
        }
    }
    game.set_origin(&id, position)?;
    let owned = game
        .entity_ref(&id)
        .map(|entity| entity.actor.clone())
        .ok_or_else(|| q1_error("Missing Q1 horde monster"))?;
    game.host.walk_move(&owned, 0.0, 0.0);
    game.update_entity(&id, |entity| {
        entity.fields.insert(String::from("category"), String::from("monster"));
        entity.owner = Some(owner.clone());
        entity.yaw_speed = 20.0;
        entity.movement_flags |= 32;
    })?;
    game.set_damageable(&id, true)?;
    let target = horde_target(game)?;
    game.update_entity(&id, |entity| {
        if let Some(monster) = entity.monster.as_mut() {
            monster.enemy = target.clone();
        }
    })?;
    let source_die = game
        .entity_ref(&id)
        .and_then(|entity| callback_name(entity.die.as_ref()))
        .ok_or_else(|| q1_error("Horde monster has no named source death"))?;
    game.update_entity(&id, |entity| {
        entity.fields.insert(String::from("horde.sourceDie"), source_die);
    })?;
    let die = game.named.die("mg1:horde:die")?;
    game.update_entity(&id, |entity| entity.die = Some(die))?;
    if kind == HordeMonster::Zombie {
        game.total_monsters -= 1;
    }
    horde_schedule(game, &id, "found", 0.1)?;
    let bounds = game.body(&id)?.bounds;
    let death = game.create("teledeath", None, None)?;
    let touch = game.named.touch("tdeath_touch")?;
    game.update_entity(&death, |entity| {
        entity.owner = Some(id.clone());
        entity.solid = Q1Solid::Trigger;
        entity.touch = Some(touch);
    })?;
    game.set_body(
        &death,
        &BodyPatch {
            origin: Some(position),
            bounds: Some(Bounds {
                min: vsub(bounds.min, Vec3 { x: 1.0, y: 1.0, z: 1.0 }),
                max: vadd(bounds.max, Vec3 { x: 1.0, y: 1.0, z: 1.0 }),
            }),
            ..Default::default()
        },
    )?;
    game.link(&death)?;
    let enclosure = game.body(&death)?.bounds;
    for observation in game.host.actors.observations() {
        let Some(body) = game.host.bodies.read(&observation.id) else {
            continue;
        };
        let death_box = Bounds {
            min: vadd(position, enclosure.min),
            max: vadd(position, enclosure.max),
        };
        let actor_box = Bounds {
            min: vadd(body.origin, body.bounds.min),
            max: vadd(body.origin, body.bounds.max),
        };
        if overlaps(&death_box, &actor_box) {
            game.invoke_touch(&death, &observation.id, None, None)?;
        }
    }
    game.schedule(&death, 0.01, "SUB_Remove")?;
    Ok(id)
}

/// Prepare a wave (`prepare`).
fn horde_prepare(game: &mut Q1EntityServices, manager: &ActorId) -> Result<bool, Q1Error> {
    set_addon_number(game, manager, "key_spawned", 0.0)?;
    let players = horde_living_players(game).len();
    if players < 1 {
        return Ok(false);
    }
    let caches = game
        .entity_ids()
        .into_iter()
        .filter(|id| {
            game.entity_ref(id)
                .is_some_and(|entity| entity.classname == "info_horde_item" && entity.wait == 0.0)
        })
        .collect::<Vec<_>>();
    for cache in &caches {
        let delay = game.host.random() * 2.0;
        horde_schedule(game, cache, "item", delay)?;
    }
    let wave = number(game, manager, "wave") + 1.0;
    let skill = game.options().skill;
    let level = wave
        + if skill >= 3 {
            6.0
        } else if skill >= 2 {
            3.0
        } else {
            0.0
        };
    let army = (level + 2.0) % 3.0 == 0.0;
    set_addon_number(game, manager, "wave", wave)?;
    set_addon_number(game, manager, "army", if army { 1.0 } else { 0.0 })?;
    let scale = if players >= 4 {
        2.0
    } else if players >= 3 {
        1.5
    } else if players >= 2 {
        1.25
    } else {
        1.0
    };
    let mut bosses = number(game, manager, "bosses");
    if level % 3.0 == 0.0 {
        bosses = ((level + 1.0) / 4.0).floor();
    } else if skill > 1 && !army && level > 9.0 {
        bosses = ((level + 1.0) / 8.0).floor();
    }
    let elites = ((((level - 1.0) / 3.0).ceil() - (bosses / 2.0).floor()) * scale).floor();
    let fodder = ((level + 2.0 - (bosses * 2.0 + elites)) * scale).floor();
    set_addon_number(game, manager, "bosses", bosses)?;
    set_addon_number(game, manager, "elites", elites)?;
    set_addon_number(game, manager, "fodder", fodder)?;
    let (target, actor, activator) = game
        .entity_ref(manager)
        .map(|entity| {
            (
                entity.target.clone(),
                entity.actor.id().clone(),
                entity.activator.clone(),
            )
        })
        .ok_or_else(|| q1_error("Missing Q1 horde manager"))?;
    for target_id in game.find(&target) {
        let name = game
            .entity_ref(&target_id)
            .and_then(|entity| entity.use_callback.clone());
        if let Some(name) = name {
            game.invoke_use(&target_id, &name, Some(&actor), activator.as_ref())?;
        }
    }
    game.update_entity(manager, |entity| entity.wait = 1.0)?;
    Ok(true)
}

/// Spawn one wave squad (`spawnWave`).
fn horde_spawn_wave(game: &mut Q1EntityServices, manager: &ActorId) -> Result<(), Q1Error> {
    let (category, word) = if number(game, manager, "fodder") > 0.0 {
        (HordeCategory::Fodder, "fodder")
    } else if number(game, manager, "elites") > 0.0 {
        (HordeCategory::Elites, "elites")
    } else {
        (HordeCategory::Bosses, "bosses")
    };
    let army = number(game, manager, "army") != 0.0;
    let selection = choose_horde_squad(army, category, &mut || game.host.random());
    let Some((squad, squad_type)) = selection else {
        set_addon_number(game, manager, "bosses", 0.0)?;
        return horde_schedule(game, manager, "wave", 1.0);
    };
    let Some(point) = horde_find_spawn(game, squad_type)? else {
        return horde_schedule(game, manager, "wave", 1.0);
    };
    let wait_until = game.time + 5.0;
    game.update_entity(&point, |entity| entity.wait = wait_until)?;
    let activator = game.entity_ref(manager).and_then(|entity| entity.activator.clone());
    game.use_targets(&point, activator.as_ref())?;
    let body = game.body(&point)?;
    let skill = game.options().skill;
    let spawns = horde_squad(squad, skill, &mut || game.host.random());
    for spawn in &spawns {
        horde_spawn_monster(
            game,
            spawn.monster,
            vadd(body.origin, spawn.offset),
            body.angles,
            manager,
        )?;
    }
    game.effect(Q1Effect::Teleport, body.origin, None, 1);
    let remaining = number(game, manager, word) - 1.0;
    set_addon_number(game, manager, word, remaining)?;
    if number(game, manager, "fodder") + number(game, manager, "elites") + number(game, manager, "bosses") <= 0.0 {
        game.update_entity(manager, |entity| entity.wait = 0.0)?;
        return horde_schedule(game, manager, "check", 30.0);
    }
    let delay = 2.0 + game.host.random();
    horde_schedule(game, manager, "wave", delay)
}

/// Check wave completion (`checkWave`).
fn horde_check_wave(game: &mut Q1EntityServices, manager: &ActorId) -> Result<(), Q1Error> {
    if game.entity_ref(manager).map(|entity| entity.wait).unwrap_or(0.0) != 0.0 {
        return Ok(());
    }
    horde_schedule(game, manager, "check", 10.0)?;
    if game.killed_monsters + 3 >= game.total_monsters {
        let cleared = game
            .entity_ids()
            .into_iter()
            .filter(|id| {
                game.entity_ref(id)
                    .is_some_and(|entity| entity.text("category") == "monster")
                    && game.health(id) <= 0.0
            })
            .collect::<Vec<_>>();
        for id in &cleared {
            game.update_entity(id, |entity| {
                entity.fields.insert(String::from("category"), String::new());
            })?;
        }
    }
    let wave = number(game, manager, "wave");
    let threshold = if wave % 3.0 == 0.0 || wave < 3.0 { 0 } else { 5 };
    if horde_living_monsters(game).len() > threshold {
        return Ok(());
    }
    for player in (game.host.players)() {
        if with_horde_services(game, |services| services.dead_flag(&player))? > 0 {
            with_horde_services(game, |services| services.respawn_teammate(&player))?;
        }
    }
    game.update_entity(manager, |entity| entity.wait = 1.0)?;
    if wave % 3.0 == 0.0 {
        if number(game, manager, "key_spawned") == 0.0 {
            horde_get_key(game, manager)?;
        }
        return horde_schedule(game, manager, "countdown", 20.0);
    }
    horde_schedule(game, manager, "countdown", 0.0)
}

/// Recheck the wave from a monster death (`remoteWavecheck`).
fn horde_remote_wavecheck(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    let Some(manager) = horde_manager(game) else {
        return Ok(());
    };
    if game.intermission.is_some() {
        return Ok(());
    }
    horde_check_wave(game, &manager)
}

/// Spawn the wave key (`getKey`).
fn horde_get_key(game: &mut Q1EntityServices, manager: &ActorId) -> Result<(), Q1Error> {
    let points = game
        .entity_ids()
        .into_iter()
        .filter(|id| {
            game.entity_ref(id)
                .is_some_and(|entity| entity.classname == "info_horde_key")
        })
        .collect::<Vec<_>>();
    let Some(first) = points.first().cloned() else {
        return horde_schedule(game, manager, "countdown", 4.0);
    };
    let wave = number(game, manager, "wave");
    let flag = if wave <= 3.0 {
        1
    } else if wave <= 6.0 {
        2
    } else if wave <= 9.0 {
        4
    } else {
        8
    };
    let point = points
        .iter()
        .find(|id| game.entity_ref(id).is_some_and(|entity| entity.spawnflags & flag != 0))
        .cloned();
    match point {
        Some(point) => horde_schedule(game, &point, if flag == 4 { "gold" } else { "silver" }, 0.0),
        None => horde_schedule(game, &first, if wave == 9.0 { "gold" } else { "silver" }, 0.0),
    }
}

/// Restore horde keys on admission (`restoreKeys`).
pub(crate) fn horde_restore_keys(game: &mut Q1EntityServices, player: &ActorId) -> Result<(), Q1Error> {
    let (Some(manager), Some(actor)) = (horde_manager(game), game.host.actors.resolve_owned(player)) else {
        return Ok(());
    };
    for key in ["silver", "gold"] {
        if number(game, &manager, &format!("keys_{key}")) > 0.0 {
            game.host.inventory.give(&actor, &format!("q1:key/{key}"), 1.0);
        }
    }
    Ok(())
}

/// Change the shared horde key count (`changeKeys`).
pub(crate) fn horde_change_keys(game: &mut Q1EntityServices, key: &str, delta: i32) -> Result<(), Q1Error> {
    let Some(manager) = horde_manager(game) else {
        return Err(q1_error("Horde keys require their manager"));
    };
    let count = number(game, &manager, &format!("keys_{key}")) + f64::from(delta);
    set_addon_number(game, &manager, &format!("keys_{key}"), count)?;
    if delta == 1 && count == 1.0 || delta == -1 && count == 0.0 {
        for player in (game.host.players)() {
            let Some(actor) = game.host.actors.resolve_owned(&player) else {
                continue;
            };
            if delta == 1 {
                game.host.inventory.give(&actor, &format!("q1:key/{key}"), 1.0);
            } else {
                game.host.inventory.consume(&actor, &format!("q1:key/{key}"), 1.0);
            }
        }
    }
    Ok(())
}

/// Score a teammate kill (`teammateKilled`).
pub fn horde_teammate_killed(game: &mut Q1EntityServices, attacker: &ActorId) -> Result<(), Q1Error> {
    if addon_cvar(game, "horde")? == 0.0 {
        return Ok(());
    }
    set_addon_player_number(game, attacker, "killtime", 0.0)?;
    with_horde_services(game, |services| services.add_score(attacker, -2.0))
}

/// Horde respawn gate (`requestRespawn`): true consumes ordinary respawn.
pub fn horde_request_respawn(game: &mut Q1EntityServices) -> Result<bool, Q1Error> {
    if horde_manager(game).is_none() {
        return Ok(false);
    }
    if !game.options().coop || horde_living_players(game).is_empty() {
        horde_restart_after_defeat(game)?;
    }
    Ok(true)
}

/// Restart after defeat (`restartAfterDefeat`).
pub fn horde_restart_after_defeat(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    let Some(manager) = horde_manager(game) else {
        return Err(q1_error("Horde restart requires its manager"));
    };
    let flags = number(game, &manager, "horde.startingFlags") as i32;
    update_base(game, |state| state.campaign.write_flags(flags))?;
    let map = game.map_name.clone();
    with_horde_services(game, |services| services.restart_session(&map, flags))
}

fn horde_kill_rule(monster: &BaseMonster) -> bool {
    if monster
        .game
        .entity_ref(&monster.id)
        .is_some_and(|entity| entity.classname != "monster_zombie")
    {
        return true;
    }
    horde_manager(monster.game).is_none()
}

fn horde_attack_delay(game: &mut Q1EntityServices, player: &ActorId, delay: f64) -> Result<f64, Q1Error> {
    if game.player_ref(player).map(|state| state.weapon) != Some(Q1Weapon::Axe)
        || addon_cvar(game, "horde")? == 0.0
        || update_base(game, |state| state.campaign.read_flags())? & 4 == 0
    {
        return Ok(delay);
    }
    let chain = addon_player_number(game, player, "axe_hit_chain")?;
    let chop = chain >= 2.0 && game.time < addon_player_number(game, player, "axe_hit_chain_time")?;
    game.update_player(player, |state| state.weapon_animation_base = if chop { 1 } else { 5 })?;
    Ok(if chop {
        0.8
    } else if chain > 1.0 {
        0.6
    } else {
        0.4
    })
}

fn horde_attach(game: &mut Q1EntityServices, player: &ActorId) -> Result<(), Q1Error> {
    horde_restore_keys(game, player)
}

fn horde_after_physics(game: &mut Q1EntityServices, player: &ActorId, _seconds: f64) -> Result<(), Q1Error> {
    let spree = addon_player_number(game, player, "killspree")?;
    if spree > 0.0 && game.time > addon_player_number(game, player, "killtime")? {
        if spree > 1.0 {
            let score = (spree * spree / 2.0).ceil();
            game.message(
                Some(player),
                "$qc_horde_streak_ended",
                false,
                vec![Q1MessageArg::Number(score)],
            );
            with_horde_services(game, |services| services.add_score(player, score))?;
        }
        set_addon_player_number(game, player, "killspree", 0.0)?;
    }
    Ok(())
}

fn head_fade_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if addon_cvar(game, "horde")? == 0.0 {
        game.cancel(id);
        return Ok(());
    }
    if number(game, id, "alpha") == 0.0 {
        addon_alpha(game, id, 1.0)?;
    }
    let delay = 10.0 + game.host.random() * 5.0;
    horde_schedule(game, id, "head_fade_step", delay)
}

fn head_fade_step_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if number(game, id, "alpha") <= 0.0 {
        return game.remove(id);
    }
    let alpha = fround(number(game, id, "alpha") - addon_frame_time(game)?);
    addon_alpha(game, id, alpha)?;
    horde_schedule(game, id, "head_fade_step", 0.0)
}

fn set_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    horde_schedule(game, id, "countdown", 1.0)
}

fn countdown_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if !horde_prepare(game, id)? {
        return Ok(());
    }
    addon_broadcast(game, "3");
    game.sound(id, "misc/talk.wav", Q1SoundChannel::Voice, 0.0, 1.0)?;
    horde_schedule(game, id, "countdown2", 1.0)
}

fn countdown2_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    addon_broadcast(game, "2");
    game.sound(id, "misc/talk.wav", Q1SoundChannel::Voice, 0.0, 1.0)?;
    horde_schedule(game, id, "countdown3", 1.0)
}

fn countdown3_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    addon_broadcast(game, "1");
    game.sound(id, "misc/talk.wav", Q1SoundChannel::Voice, 0.0, 1.0)?;
    horde_schedule(game, id, "fight", 1.0)
}

fn fight_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    addon_broadcast(
        game,
        if number(game, id, "wave") % 3.0 == 0.0 {
            "$qc_horde_boss_wave"
        } else {
            "$qc_horde_fight"
        },
    );
    horde_spawn_wave(game, id)?;
    game.sound(id, "misc/talk.wav", Q1SoundChannel::Voice, 0.0, 1.0)
}

fn wave_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    horde_spawn_wave(game, id)
}

fn check_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    horde_check_wave(game, id)
}

fn check_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    horde_check_wave(game, id)
}

fn point_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| entity.spawnflags ^= 1)
}

fn door_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    normal: Option<Vec3>,
    surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    if horde_manager(game).is_none() {
        let fallback = game.named.touch_handler("door_touch")?;
        return fallback(game, id, other, normal, surface);
    }
    if !game.is_player(other) {
        return Ok(());
    }
    let master = game
        .entity_ref(id)
        .and_then(|entity| entity.door_group.first().cloned())
        .unwrap_or_else(|| id.clone());
    if game
        .entity_ref(&master)
        .map(|entity| entity.attack_finished)
        .unwrap_or(0.0)
        > game.time
    {
        return Ok(());
    }
    let ready = game.time + 2.0;
    game.update_entity(&master, |entity| entity.attack_finished = ready)?;
    let message = game
        .entity_ref(&master)
        .map(|entity| entity.message.clone())
        .unwrap_or_default();
    if !message.is_empty() {
        game.message_simple(Some(other), &message);
        if game.host.actors.resolve_owned(other).is_some() {
            game.sound(other, "misc/talk.wav", Q1SoundChannel::Voice, 1.0, 1.0)?;
        }
    }
    let spawnflags = game.entity_ref(id).map(|entity| entity.spawnflags).unwrap_or(0);
    let silver = spawnflags & 16 != 0;
    let gold = spawnflags & 8 != 0;
    if !silver && !gold {
        return Ok(());
    }
    if silver && game.host.inventory.count(other, &String::from("q1:key/silver")) == 0.0
        || gold && game.host.inventory.count(other, &String::from("q1:key/gold")) == 0.0
    {
        let path = if game.world_type == 2 {
            "doors/basetry.wav"
        } else if game.world_type == 1 {
            "doors/runetry.wav"
        } else {
            "doors/medtry.wav"
        };
        game.sound(id, path, Q1SoundChannel::Voice, 1.0, 1.0)?;
        if silver != gold {
            let key = if game.world_type == 2 {
                "keycard"
            } else if game.world_type == 1 {
                "runekey"
            } else {
                "key"
            };
            game.message_simple(
                Some(other),
                &format!("$qc_need_{}_{key}", if silver { "silver" } else { "gold" }),
            );
        }
        return Ok(());
    }
    horde_change_keys(game, if silver { "silver" } else { "gold" }, -1)?;
    let doors = game
        .entity_ref(&master)
        .map(|entity| entity.door_group.clone())
        .unwrap_or_default();
    let doors = if doors.is_empty() { vec![master.clone()] } else { doors };
    for door in &doors {
        game.update_entity(door, |entity| entity.touch = None)?;
    }
    game.invoke_use(&master, "door_use", Some(other), Some(other))
}

fn found_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if let Some(mut controller) = ordinary_addon_monster(game, id)? {
        match controller.enemy() {
            None => {
                let stand = controller.spec.stand;
                controller.play(stand)?;
            }
            Some(enemy) => controller.found(&enemy)?,
        }
        controller.finish()?;
        return Ok(());
    }
    let native = game
        .entity_ref(id)
        .and_then(|entity| entity.monster.clone())
        .is_some_and(|monster| matches!(monster.species, Q1MonsterSpecies::Army | Q1MonsterSpecies::Dog));
    if native {
        return game.invoke_action(id, "monster_found_target");
    }
    Err(q1_error("Horde monster lost its source controller"))
}

fn die_handler(game: &mut Q1EntityServices, id: &ActorId, attacker: Option<&ActorId>) -> Result<(), Q1Error> {
    if number(game, id, "horde.countedDeath") != 0.0 {
        return Ok(());
    }
    set_addon_number(game, id, "horde.countedDeath", 1.0)?;
    if attacker.is_some_and(|attacker| game.is_player(attacker)) {
        let attacker = attacker.expect("player");
        let spree = addon_player_number(game, attacker, "killspree")? + 1.0;
        set_addon_player_number(game, attacker, "killspree", spree)?;
        set_addon_player_number(game, attacker, "killtime", game.time + 2.0)?;
        if spree > 1.0 {
            if spree >= 14.0 {
                game.message(
                    Some(attacker),
                    "$qc_horde_streak_generic",
                    false,
                    vec![Q1MessageArg::Number(spree)],
                );
            } else {
                game.message(Some(attacker), &format!("$qc_horde_streak_{spree}"), false, Vec::new());
            }
        }
    }
    spawn_horde_powerup(game, id)?;
    if attacker.is_some_and(|attacker| game.is_player(attacker)) {
        let attacker = attacker.expect("player");
        with_horde_services(game, |services| services.add_score(attacker, 1.0))?;
    }
    // The base controller owns the kill counter. Horde zombies are excluded by its source hook.
    let source = game
        .entity_ref(id)
        .map(|entity| entity.text("horde.sourceDie"))
        .unwrap_or_default();
    let die = game.named.die_handler(&source)?;
    die(game, id, attacker)?;
    let head = game
        .entity_ref(id)
        .is_some_and(|entity| entity.movement == Q1MoveType::Bounce && entity.model.starts_with("progs/h_"));
    if game.is_live(id) && head {
        horde_schedule(game, id, "head_fade", 1.0)?;
    }
    if game
        .entity_ref(id)
        .map(|entity| entity.classname.clone())
        .unwrap_or_default()
        == "monster_zombie"
    {
        return Ok(());
    }
    horde_remote_wavecheck(game)
}

fn spawn_horde_manager(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if addon_cvar(game, "horde")? == 0.0 && game.options().deathmatch == 0 {
        addon_set_cvar(game, "horde", "1")?;
    }
    let flags = update_base(game, |state| state.campaign.read_flags())?;
    game.update_entity(id, |entity| {
        if entity.target.is_empty() {
            entity.target = String::from("horde_event");
        }
        entity.targetname = String::from("horde_manager");
        entity.wait = 1.0;
        entity.delay = 9.0;
    })?;
    set_addon_number(game, id, "wave", 0.0)?;
    set_addon_number(game, id, "horde.startingFlags", f64::from(flags))?;
    let use_callback = game.named.use_callback("mg1:horde:check")?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_callback))?;
    horde_schedule(game, id, "set", 10.0)
}

fn spawn_monster_start(game: &mut Q1EntityServices, id: &ActorId, width: f32) -> Result<(), Q1Error> {
    let use_callback = game.named.use_callback("mg1:horde:point")?;
    game.update_entity(id, |entity| {
        entity.wait = 0.0;
        entity.use_callback = Some(use_callback);
    })?;
    game.set_bounds(
        id,
        Bounds {
            min: Vec3 {
                x: -width,
                y: -width,
                z: 0.0,
            },
            max: Vec3 {
                x: width,
                y: width,
                z: 128.0,
            },
        },
    )
}

fn spawn_start_normal(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    spawn_monster_start(game, id, 80.0)
}

fn spawn_start_flying(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    spawn_monster_start(game, id, 80.0)
}

fn spawn_start_ranged(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    spawn_monster_start(game, id, 44.0)
}

fn spawn_start_boss(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    spawn_monster_start(game, id, 44.0)
}

fn spawn_horde_door(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    spawn_door(game, id)?;
    let touch = game.named.touch("mg1:horde:door_touch")?;
    game.update_entity(id, |entity| entity.touch = Some(touch))
}

/// Registers the Q1 horde (`registerQ1Horde`). Requires the MG1 or
/// DOPA source program.
pub fn register_q1_horde(game: &mut Q1EntityServices, services: Box<dyn Q1HordeServices>) -> Result<(), Q1Error> {
    if !matches!(addon_program(game)?, Q1AddonProgram::Mg1 | Q1AddonProgram::Dopa) {
        return Err(q1_error("Official horde requires the MG1 source program"));
    }
    register_horde_state(game, services);
    register_kill_count_rule(game, "mg1:horde", horde_kill_rule)?;
    game.register_weapon_rules(Q1WeaponRules {
        id: String::from("mg1:horde"),
        attack_delay: Some(horde_attack_delay),
        ..Default::default()
    })?;
    game.named.register(
        "mg1:horde:head_fade",
        Q1CallbackHandlers {
            action: Some(head_fade_action as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        "mg1:horde:head_fade_step",
        Q1CallbackHandlers {
            action: Some(head_fade_step_action as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        "mg1:horde:set",
        Q1CallbackHandlers {
            action: Some(set_action as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        "mg1:horde:countdown",
        Q1CallbackHandlers {
            action: Some(countdown_action as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        "mg1:horde:countdown2",
        Q1CallbackHandlers {
            action: Some(countdown2_action as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        "mg1:horde:countdown3",
        Q1CallbackHandlers {
            action: Some(countdown3_action as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        "mg1:horde:fight",
        Q1CallbackHandlers {
            action: Some(fight_action as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        "mg1:horde:wave",
        Q1CallbackHandlers {
            action: Some(wave_action as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        "mg1:horde:check",
        Q1CallbackHandlers {
            action: Some(check_action as Q1ActionHandler),
            use_callback: Some(check_use as Q1UseHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        "mg1:horde:point",
        Q1CallbackHandlers {
            use_callback: Some(point_use as Q1UseHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        "mg1:horde:door_touch",
        Q1CallbackHandlers {
            touch: Some(door_touch as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        "mg1:horde:found",
        Q1CallbackHandlers {
            action: Some(found_action as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        "mg1:horde:die",
        Q1CallbackHandlers {
            die: Some(die_handler as Q1DieHandler),
            ..Default::default()
        },
    )?;
    game.register_spawn("func_door", spawn_horde_door)?;
    game.register_spawn("horde_manager", spawn_horde_manager)?;
    game.register_spawn("info_monster_start", spawn_start_normal)?;
    game.register_spawn("info_monster_start_flying", spawn_start_flying)?;
    game.register_spawn("info_monster_start_ranged", spawn_start_ranged)?;
    game.register_spawn("info_monster_start_boss", spawn_start_boss)?;
    game.register_player_extension(Q1PlayerExtension {
        id: String::from("mg1:horde"),
        attach: Some(horde_attach),
        after_physics: Some(horde_after_physics),
        ..Default::default()
    })?;
    register_horde_loot(game)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::q1::addons::context::{attach_test_player, register_test_addons};
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::missionpacks::types::test_game;

    #[derive(Default)]
    struct TestHordeServices {
        scores: Arc<Mutex<Vec<f64>>>,
        restarts: Arc<Mutex<Vec<(String, i32)>>>,
    }

    impl Q1HordeServices for TestHordeServices {
        fn dead_flag(&mut self, _player: &ActorId) -> i32 {
            0
        }
        fn no_target(&mut self, _player: &ActorId) -> bool {
            false
        }
        fn is_bot(&mut self, _player: &ActorId) -> bool {
            false
        }
        fn respawn_teammate(&mut self, _player: &ActorId) {}
        fn add_score(&mut self, _player: &ActorId, delta: f64) {
            self.scores.lock().expect("scores").push(delta);
        }
        fn restart_session(&mut self, map: &str, starting_server_flags: i32) {
            self.restarts
                .lock()
                .expect("restarts")
                .push((map.to_string(), starting_server_flags));
        }
    }

    fn setup(game: &mut Q1EntityServices) -> (Q1BaseGuard, TestHordeServices) {
        let guard = Q1BaseGuard::register(game, Q1BaseOptions::default()).expect("base");
        register_test_addons(game, Q1AddonProgram::Mg1);
        let services = TestHordeServices::default();
        let shared = TestHordeServices {
            scores: Arc::clone(&services.scores),
            restarts: Arc::clone(&services.restarts),
        };
        register_q1_horde(game, Box::new(shared)).expect("horde");
        (guard, services)
    }

    fn spawn_manager(game: &mut Q1EntityServices) -> ActorId {
        let manager = game.create("horde_manager", None, None).expect("manager");
        game.spawn_entity(&manager, None).expect("spawn");
        manager
    }

    /// Admit players through the host hook (the mock host starts empty).
    fn admit(game: &mut Q1EntityServices, players: Vec<ActorId>) {
        game.host.players = Box::new(move || players.clone());
    }

    /// Configure empty key entries (mock `give` needs a bound entry).
    fn keyed(game: &mut Q1EntityServices, player: &ActorId) {
        use crate::contract::InventoryEntry;

        let owned = game.player_owned(player).expect("owned");
        for key in ["silver", "gold"] {
            game.host
                .inventory
                .configure(
                    &owned,
                    &InventoryEntry {
                        item: format!("q1:key/{key}"),
                        count: 0.0,
                        capacity: 1.0,
                        count_policy: None,
                    },
                )
                .expect("key entry");
        }
    }

    #[test]
    fn program_gate_rejects_ctf() {
        let mut game = test_game();
        let _guard = Q1BaseGuard::register(&mut game, Q1BaseOptions::default()).expect("base");
        register_test_addons(&mut game, Q1AddonProgram::Ctf);
        assert!(register_q1_horde(&mut game, Box::new(TestHordeServices::default())).is_err());
    }

    #[test]
    fn countdown_prepares_first_wave() {
        let mut game = test_game();
        let (_guard, _services) = setup(&mut game);
        let manager = spawn_manager(&mut game);
        assert_eq!(game.entity_ref(&manager).expect("manager").target, "horde_event");
        assert_eq!(number(&game, &manager, "wave"), 0.0);
        game.invoke_action(&manager, "mg1:horde:set").expect("set");
        let player = attach_test_player(&mut game);
        admit(&mut game, vec![player]);
        game.invoke_action(&manager, "mg1:horde:countdown").expect("countdown");
        assert_eq!(number(&game, &manager, "wave"), 1.0);
        assert_eq!(game.entity_ref(&manager).expect("manager").wait, 1.0);
        assert_eq!(
            game.entity_ref(&manager).expect("manager").think.as_deref(),
            Some("mg1:horde:countdown2")
        );
        let remaining =
            number(&game, &manager, "fodder") + number(&game, &manager, "elites") + number(&game, &manager, "bosses");
        assert!(remaining > 0.0);
    }

    #[test]
    fn countdown_waits_without_players() {
        let mut game = test_game();
        let (_guard, _services) = setup(&mut game);
        let manager = spawn_manager(&mut game);
        game.invoke_action(&manager, "mg1:horde:countdown").expect("countdown");
        assert_eq!(number(&game, &manager, "wave"), 0.0);
    }

    #[test]
    fn keys_round_trip_through_manager() {
        let mut game = test_game();
        let (_guard, _services) = setup(&mut game);
        let manager = spawn_manager(&mut game);
        let player = attach_test_player(&mut game);
        keyed(&mut game, &player);
        admit(&mut game, vec![player.clone()]);
        horde_change_keys(&mut game, "silver", 1).expect("grant");
        assert_eq!(number(&game, &manager, "keys_silver"), 1.0);
        assert_eq!(game.host.inventory.count(&player, &String::from("q1:key/silver")), 1.0);
        let late = attach_test_player(&mut game);
        keyed(&mut game, &late);
        horde_restore_keys(&mut game, &late).expect("restore");
        assert_eq!(game.host.inventory.count(&late, &String::from("q1:key/silver")), 1.0);
        admit(&mut game, vec![player.clone(), late.clone()]);
        horde_change_keys(&mut game, "silver", -1).expect("consume");
        assert_eq!(number(&game, &manager, "keys_silver"), 0.0);
        assert_eq!(game.host.inventory.count(&player, &String::from("q1:key/silver")), 0.0);
        assert_eq!(game.host.inventory.count(&late, &String::from("q1:key/silver")), 0.0);
    }

    #[test]
    fn keys_require_manager() {
        let mut game = test_game();
        let (_guard, _services) = setup(&mut game);
        assert!(horde_change_keys(&mut game, "gold", 1).is_err());
    }

    #[test]
    fn axe_chain_shortens_delay() {
        let mut game = test_game();
        let (_guard, _services) = setup(&mut game);
        let player = attach_test_player(&mut game);
        assert_eq!(horde_attack_delay(&mut game, &player, 0.5).expect("gated"), 0.5);
        addon_set_cvar(&game, "horde", "1").expect("cvar");
        update_base(&game, |state| state.campaign.write_flags(4)).expect("flags");
        assert_eq!(horde_attack_delay(&mut game, &player, 0.5).expect("fresh"), 0.4);
        set_addon_player_number(&mut game, &player, "axe_hit_chain", 2.0).expect("chain");
        let window = game.time + 5.0;
        set_addon_player_number(&mut game, &player, "axe_hit_chain_time", window).expect("window");
        assert_eq!(horde_attack_delay(&mut game, &player, 0.5).expect("chop"), 0.8);
        assert_eq!(game.player_ref(&player).expect("player").weapon_animation_base, 1);
    }

    #[test]
    fn streak_bonus_expires() {
        let mut game = test_game();
        let (_guard, services) = setup(&mut game);
        let player = attach_test_player(&mut game);
        set_addon_player_number(&mut game, &player, "killspree", 3.0).expect("spree");
        let expired = game.time - 1.0;
        set_addon_player_number(&mut game, &player, "killtime", expired).expect("killtime");
        horde_after_physics(&mut game, &player, 0.1).expect("physics");
        assert_eq!(*services.scores.lock().expect("scores"), vec![5.0]);
        assert_eq!(addon_player_number(&game, &player, "killspree").expect("spree"), 0.0);
    }

    #[test]
    fn defeat_restarts_session() {
        let mut game = test_game();
        let (_guard, services) = setup(&mut game);
        let manager = spawn_manager(&mut game);
        assert!(horde_request_respawn(&mut game).expect("respawn"));
        assert_eq!(services.restarts.lock().expect("restarts").len(), 1);
        assert_eq!(
            number(&game, &manager, "horde.startingFlags"),
            f64::from(services.restarts.lock().expect("restarts")[0].1)
        );
    }

    #[test]
    fn respawn_passes_without_manager() {
        let mut game = test_game();
        let (_guard, _services) = setup(&mut game);
        assert!(!horde_request_respawn(&mut game).expect("respawn"));
    }
}
