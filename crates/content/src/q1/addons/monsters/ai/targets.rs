//! Q1 mg3 path control targets (`src/content/q1/addons/monsters/ai/targets.ts`).
//!
//! `quakec_mg3/ai.qc` path_corner and path control targets.
//! GPL-2.0-or-later.

use qa_core::identity::{same_actor, ActorId};
use qa_core::math::{Bounds, Vec3};

use crate::q1::addons::context::Q1AddonContext;
use crate::q1::foundation::callbacks::{Q1CallbackHandlers, Q1TouchHandler, Q1UseHandler};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::types::{vsub, yaw_for, Q1Solid, Q1SoundChannel, ZERO};
use crate::q1::{q1_error, Q1Error};

fn same(first: Option<&ActorId>, second: Option<&ActorId>) -> bool {
    match (first, second) {
        (None, None) => true,
        (Some(first), Some(second)) => same_actor(first, second),
        _ => false,
    }
}

/// Retarget a mover at a path corner (`destination`).
fn destination(game: &mut Q1EntityServices, mover: &ActorId, name: &str) -> Result<Option<ActorId>, Q1Error> {
    let target = game.find(name).first().cloned();
    let mover_origin = game.body(mover).map(|body| body.origin)?;
    let target_origin = match target.as_ref() {
        Some(target) => game.body(target).map(|body| body.origin)?,
        None => ZERO,
    };
    let yaw = yaw_for(vsub(target_origin, mover_origin));
    game.update_entity(mover, |entity| {
        entity.references.insert(String::from("goalentity"), target.clone());
        entity.references.insert(String::from("movetarget"), target.clone());
        if let Some(monster) = entity.monster.as_mut() {
            monster.path = if target.is_some() {
                name.to_string()
            } else {
                String::new()
            };
        }
        entity.ideal_yaw = yaw;
    })?;
    Ok(target)
}

/// Pause a mover until a time (`pause`).
fn pause(game: &mut Q1EntityServices, mover: &ActorId, until: f64) -> Result<(), Q1Error> {
    let value = f64::from(until as f32);
    game.update_entity(mover, |entity| {
        entity.fields.insert(String::from("pausetime"), value.to_string());
        if let Some(monster) = entity.monster.as_mut() {
            monster.pause_until = value;
        }
    })
}

/// Advance a mover through a path corner (`moveTarget`).
fn move_target(
    game: &mut Q1EntityServices,
    corner: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    let mover = other.clone();
    let Some(entity) = game.entity_ref(&mover).cloned() else {
        return Ok(());
    };
    let movetarget = entity.references.get("movetarget").and_then(|target| target.as_ref());
    if !same(movetarget, Some(corner)) {
        return Ok(());
    }
    let enemy = entity.monster.as_ref().and_then(|monster| monster.enemy.clone());
    let referenced = entity.references.get("enemy").and_then(|enemy| enemy.clone());
    if enemy.or(referenced).is_some() {
        return Ok(());
    }
    game.update_entity(corner, |entity| {
        entity.owner = Some(mover.clone());
    })?;
    let inflictor = entity.references.get("dmg_inflictor").and_then(|id| id.clone());
    let clear_owner = inflictor
        .as_ref()
        .and_then(|id| game.entity_ref(id))
        .and_then(|previous| {
            if previous.classname == "path_corner" && same(previous.owner.as_ref(), Some(previous.actor.id())) {
                Some(previous.actor.id().clone())
            } else {
                None
            }
        });
    if let Some(previous) = clear_owner {
        game.update_entity(&previous, |entity| {
            entity.owner = None;
        })?;
    }
    game.update_entity(&mover, |entity| {
        entity
            .references
            .insert(String::from("dmg_inflictor"), Some(corner.clone()));
    })?;
    let paused = entity
        .monster
        .as_ref()
        .map(|monster| monster.pause_until)
        .unwrap_or_else(|| entity.number("pausetime"));
    if paused > game.time {
        return Ok(());
    }
    if entity.classname == "monster_ogre" {
        game.sound(&mover, "ogre/ogdrag.wav", Q1SoundChannel::Voice, 2.0, 1.0)?;
    }
    let (target_name, wait) = game
        .entity_ref(corner)
        .map(|corner| (corner.target.clone(), corner.wait))
        .unwrap_or_default();
    let target = destination(game, &mover, &target_name)?;
    if target.is_none() || wait != 0.0 {
        pause(game, &mover, game.time + if target.is_none() { 999999.0 } else { wait })?;
        if let Some(path_end) = game.entity_ref(&mover).and_then(|entity| entity.path_end.clone()) {
            game.invoke_action(&mover, &path_end)?;
        }
    }
    Ok(())
}

/// Release paused monsters at a trigger target (`cancelPause`).
fn cancel_pause(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let target = game
        .entity_ref(id)
        .map(|entity| entity.target.clone())
        .unwrap_or_default();
    for mover in game.find(&target) {
        let flags = game.entity_ref(&mover).map(|entity| entity.movement_flags).unwrap_or(0);
        if flags & 32 == 0 {
            continue;
        }
        pause(game, &mover, 0.0)?;
        let prefix = game
            .entity_ref(&mover)
            .map(|entity| entity.text("source.monsterCallbackPrefix"))
            .unwrap_or_default();
        let use_callback = game.named.use_callback(&format!("{prefix}:monster_use"))?;
        game.update_entity(&mover, |entity| {
            entity.use_callback = Some(use_callback);
        })?;
    }
    Ok(())
}

/// Retarget path corners at a trigger target (`switchPath`).
fn switch_path(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let (target, netname) = game
        .entity_ref(id)
        .map(|entity| (entity.target.clone(), entity.text("netname")))
        .unwrap_or_default();
    for corner in game.find(&target) {
        let Some(entity) = game.entity_ref(&corner).cloned() else {
            continue;
        };
        if entity.classname != "path_corner" {
            continue;
        }
        let old_target = entity.target.clone();
        game.update_entity(&corner, |entity| {
            entity.target = netname.clone();
        })?;
        let owner = game.entity_ref(&corner).and_then(|entity| entity.owner.clone());
        let Some(mover) = owner
            .as_ref()
            .and_then(|owner| game.entity_ref(owner).map(|_| owner.clone()))
        else {
            continue;
        };
        let old_goal = game.find(&old_target).first().cloned();
        let movetarget = game
            .entity_ref(&mover)
            .and_then(|entity| entity.references.get("movetarget").and_then(|target| target.clone()));
        if same(movetarget.as_ref(), old_goal.as_ref()) {
            destination(game, &mover, &netname)?;
        }
    }
    Ok(())
}

fn spawn_path_corner(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let (targetname, wait) = game
        .entity_ref(id)
        .map(|entity| (entity.targetname.clone(), entity.wait))
        .unwrap_or_default();
    if targetname.is_empty() {
        return Err(q1_error("path_corner with no targetname."));
    }
    if wait < 0.0 {
        game.update_entity(id, |entity| entity.wait = 999999.0)?;
    }
    let touch = game.named.touch("mg3:t_movetarget")?;
    game.update_entity(id, |entity| {
        entity.solid = Q1Solid::Trigger;
        entity.touch = Some(touch);
    })?;
    game.set_bounds(
        id,
        Bounds {
            min: Vec3 {
                x: -8.0,
                y: -8.0,
                z: -8.0,
            },
            max: Vec3 { x: 8.0, y: 8.0, z: 8.0 },
        },
    )
}

fn spawn_cancel_pause(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    spawn_path_trigger(game, id, "target_cancelpause")
}

fn spawn_switch_path(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    spawn_path_trigger(game, id, "target_switchpath")
}

fn spawn_path_trigger(game: &mut Q1EntityServices, id: &ActorId, classname: &str) -> Result<(), Q1Error> {
    let (target, targetname, netname) = game
        .entity_ref(id)
        .map(|entity| (entity.target.clone(), entity.targetname.clone(), entity.text("netname")))
        .unwrap_or_default();
    if target.is_empty() {
        return Err(q1_error(format!("{classname} with no target given.")));
    }
    if targetname.is_empty() {
        return Err(q1_error(format!("{classname} with no targetname given.")));
    }
    if classname == "target_switchpath" && netname.is_empty() {
        return Err(q1_error("target_switchtarget with no netname given."));
    }
    let use_callback = game.named.use_callback(&format!("mg3:{classname}_use"))?;
    game.update_entity(id, |entity| {
        entity.use_callback = Some(use_callback);
    })
}

/// Register mg3 path corner and path control targets
/// (`registerMg3PathTargets`).
pub fn register_mg3_path_targets(_context: &Q1AddonContext, game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        "mg3:t_movetarget",
        Q1CallbackHandlers {
            touch: Some(move_target as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        "mg3:target_cancelpause_use",
        Q1CallbackHandlers {
            use_callback: Some(cancel_pause as Q1UseHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        "mg3:target_switchpath_use",
        Q1CallbackHandlers {
            use_callback: Some(switch_path as Q1UseHandler),
            ..Default::default()
        },
    )?;
    game.register_spawn("path_corner", spawn_path_corner)?;
    game.register_spawn("target_cancelpause", spawn_cancel_pause)?;
    game.register_spawn("target_switchpath", spawn_switch_path)?;
    Ok(())
}
