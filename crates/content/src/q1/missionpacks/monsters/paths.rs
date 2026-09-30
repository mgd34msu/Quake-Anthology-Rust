//! Hipnotic path following (`src/content/q1/missionpacks/monsters/paths.ts`).

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

use crate::q1::base::monsters::BaseMonster;
use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::TouchSurface;
use crate::q1::foundation::types::{vsub, yaw_for, Q1MoveType, Q1Solid, Q1SoundChannel, Q1TraceRequest, POINT, ZERO};
use crate::q1::missionpacks::types::Q1MissionPack;
use crate::q1::Q1Error;

use super::runtime::{mission_pack_monsters, MissionMonster, Q1MissionPackMonsters};

/// Installed Hipnotic runtime, cloned so the static lock is never held
/// across game calls.
fn hipnotic_runtime() -> Q1MissionPackMonsters {
    mission_pack_monsters(&Q1MissionPack::Hipnotic)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

fn store_hipnotic_runtime(runtime: Q1MissionPackMonsters) {
    *mission_pack_monsters(&Q1MissionPack::Hipnotic)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = runtime;
}

fn follow_mission(monster: &mut MissionMonster, trigger_target: &str) {
    let enemy = monster.enemy.clone();
    let start = monster.eye(None);
    let end = match enemy.as_ref() {
        None => monster
            .game
            .world
            .clone()
            .and_then(|world| monster.game.body(&world).ok())
            .map(|body| body.origin),
        Some(enemy) => monster.eye(Some(enemy)),
    };
    let (Some(start), Some(end)) = (start, end) else {
        return;
    };
    let id = monster.entity.actor.id().clone();
    let trace = monster.game.host.trace(&Q1TraceRequest {
        start,
        end,
        bounds: POINT,
        ignore: Some(id.clone()),
        monsters: true,
        missile: false,
    });
    if trace.fraction == 1.0 {
        return;
    }
    if let Some(enemy) = enemy {
        monster.state.old_enemy = Some(enemy);
        monster.enemy = None;
        monster.next_frame = monster.spec.walk.to_string();
        let think = format!("{}:monster_frame", monster.prefix());
        if let Ok(think) = monster.game.named.action(&think) {
            monster.entity.think = Some(think);
        }
        monster.flush_entity();
    }
    let target = monster.game.find(trigger_target).first().cloned();
    let target_entity = target.as_ref().and_then(|target| monster.game.entity(target).cloned());
    monster.state.path = target_entity
        .as_ref()
        .map(|entity| entity.targetname.clone())
        .unwrap_or_default();
    monster
        .entity
        .references
        .insert("goalentity".to_string(), target.clone());
    monster
        .entity
        .references
        .insert("movetarget".to_string(), target.clone());
    let destination = target
        .as_ref()
        .and_then(|target| monster.game.body(target).ok())
        .map(|body| body.origin)
        .unwrap_or(ZERO);
    monster.entity.ideal_yaw = yaw_for(vsub(destination, monster.origin));
    let wet = monster.game.time + 2.0;
    monster
        .entity
        .fields
        .insert("wetsuit_time".to_string(), (wet as f32).to_string());
    monster.flush_entity();
    if target.is_some() {
        return;
    }
    if let Some(old_enemy) = monster.state.old_enemy.clone() {
        monster.found(&old_enemy);
        return;
    }
    let owned = monster.entity.actor.clone();
    let client = monster.game.host.check_client(&owned);
    if client.is_none() {
        if let Some(world) = monster.game.world.clone() {
            monster.found(&world);
            return;
        }
    }
    monster.state.pause_until = monster.game.time + 999999.0;
    let stand = monster.spec.stand;
    monster.play(stand);
}

fn follow_base(monster: &mut BaseMonster, trigger_target: &str) -> Result<(), Q1Error> {
    let enemy = monster.enemy();
    let start = monster.eye(None).unwrap_or(None);
    let end = match enemy.as_ref() {
        None => monster
            .game
            .world
            .clone()
            .and_then(|world| monster.game.body(&world).ok())
            .map(|body| body.origin),
        Some(enemy) => monster.eye(Some(enemy)).unwrap_or(None),
    };
    let (Some(start), Some(end)) = (start, end) else {
        return Ok(());
    };
    let id = monster.id.clone();
    let trace = monster.game.host.trace(&Q1TraceRequest {
        start,
        end,
        bounds: POINT,
        ignore: Some(id.clone()),
        monsters: true,
        missile: false,
    });
    if trace.fraction == 1.0 {
        return Ok(());
    }
    if let Some(enemy) = enemy {
        monster.monster.old_enemy = Some(enemy);
        monster.set_enemy(None);
        monster.controller.next_frame = monster.spec.walk.to_string();
        let think = format!("{}:monster_frame", monster.prefix);
        if let Ok(think) = monster.game.named.action(&think) {
            monster.game.update_entity(&id, |entity| entity.think = Some(think))?;
        }
    }
    let target = monster.game.find(trigger_target).first().cloned();
    let target_entity = target.as_ref().and_then(|target| monster.game.entity(target).cloned());
    monster.monster.path = target_entity
        .as_ref()
        .map(|entity| entity.targetname.clone())
        .unwrap_or_default();
    monster.game.update_entity(&id, |entity| {
        entity.references.insert("goalentity".to_string(), target.clone());
        entity.references.insert("movetarget".to_string(), target.clone());
    })?;
    let destination = target
        .as_ref()
        .and_then(|target| monster.game.body(target).ok())
        .map(|body| body.origin)
        .unwrap_or(ZERO);
    let origin = monster.origin()?;
    let yaw = yaw_for(vsub(destination, origin));
    let wet = monster.game.time + 2.0;
    monster.game.update_entity(&id, |entity| {
        entity.ideal_yaw = yaw;
        entity
            .fields
            .insert("wetsuit_time".to_string(), (wet as f32).to_string());
    })?;
    if target.is_some() {
        return Ok(());
    }
    if let Some(old_enemy) = monster.monster.old_enemy.clone() {
        monster.found(&old_enemy)?;
        return Ok(());
    }
    let owned = monster.game.entity_ref(&id).map(|entity| entity.actor.clone());
    let client = owned.as_ref().and_then(|owned| monster.game.host.check_client(owned));
    if client.is_none() {
        if let Some(world) = monster.game.world.clone() {
            monster.found(&world)?;
            return Ok(());
        }
    }
    monster.monster.pause_until = monster.game.time + 999999.0;
    let stand = monster.spec.stand;
    monster.play(stand)?;
    Ok(())
}

fn follow_touch_handler(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let trigger = game.entity(id).cloned();
    let entity = game.entity(other).cloned();
    let (Some(trigger), Some(entity)) = (trigger, entity) else {
        return Ok(());
    };
    if entity.movement_flags & 32 == 0
        || entity.classname == "monster_decoy"
        || entity.number("wetsuit_time") > game.time
    {
        return Ok(());
    }
    let owned = match game.host.actors.resolve_owned(other) {
        Some(owned) => owned,
        None => return Ok(()),
    };
    let mut runtime = hipnotic_runtime();
    if runtime.contains(&owned) {
        let mut monster = match runtime.require(game, other) {
            Ok(monster) => monster,
            Err(_) => return Ok(()),
        };
        follow_mission(&mut monster, &trigger.target);
        monster.finish();
        store_hipnotic_runtime(runtime);
        return Ok(());
    }
    let Ok(mut monster) = BaseMonster::load(game, other) else {
        return Ok(());
    };
    let result = follow_base(&mut monster, &trigger.target);
    monster.finish()?;
    result
}

fn path_follow_spawn(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let touch = game.named.touch("hipnotic:t_followtarget")?;
    game.update_entity(id, |entity| {
        entity.solid = Q1Solid::Trigger;
        entity.touch = Some(touch);
    })?;
    let classname = game
        .entity_ref(id)
        .map(|entity| entity.classname.clone())
        .unwrap_or_default();
    if classname == "path_follow2" {
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
        )?;
        return Ok(());
    }
    game.update_entity(id, |entity| {
        entity.movement = Q1MoveType::None;
        entity.model = String::new();
    })?;
    game.link(id)
}

fn movetarget_mission(monster: &mut MissionMonster, corner: &ActorId) -> bool {
    let Some(corner_entity) = monster.game.entity(corner).cloned() else {
        return false;
    };
    if monster.state.path != corner_entity.targetname || monster.enemy.is_some() {
        return true;
    }
    if monster.entity.classname == "monster_ogre" {
        let id = monster.entity.actor.id().clone();
        let _ = monster
            .game
            .sound(&id, "ogre/ogdrag.wav", Q1SoundChannel::Voice, 1.0, 2.0);
    }
    if !corner_entity.target.is_empty() {
        let target = monster.game.find(&corner_entity.target).first().cloned();
        let target_entity = target.as_ref().and_then(|target| monster.game.entity(target).cloned());
        monster.state.path = target_entity
            .as_ref()
            .map(|entity| entity.targetname.clone())
            .unwrap_or_default();
        monster
            .entity
            .references
            .insert("goalentity".to_string(), target.clone());
        monster
            .entity
            .references
            .insert("movetarget".to_string(), target.clone());
        let destination = target
            .as_ref()
            .and_then(|target| monster.game.body(target).ok())
            .map(|body| body.origin)
            .unwrap_or(ZERO);
        monster.entity.ideal_yaw = yaw_for(vsub(destination, monster.origin));
        monster.flush_entity();
        if target.is_some() {
            if corner_entity.delay != 0.0 {
                monster.state.pause_until = monster.game.time + corner_entity.delay;
                let stand = monster.spec.stand;
                monster.play(stand);
            }
            return true;
        }
    }
    monster.state.pause_until = monster.game.time + 999999.0;
    let stand = monster.spec.stand;
    monster.play(stand);
    true
}

fn movetarget_base(monster: &mut BaseMonster, corner: &ActorId) -> Result<bool, Q1Error> {
    let Some(corner_entity) = monster.game.entity(corner).cloned() else {
        return Ok(false);
    };
    if monster.monster.path != corner_entity.targetname || monster.enemy().is_some() {
        return Ok(true);
    }
    if monster
        .game
        .entity_ref(&monster.id)
        .map(|entity| entity.classname.as_str())
        == Some("monster_ogre")
    {
        let id = monster.id.clone();
        monster
            .game
            .sound(&id, "ogre/ogdrag.wav", Q1SoundChannel::Voice, 1.0, 2.0)?;
    }
    if !corner_entity.target.is_empty() {
        let target = monster.game.find(&corner_entity.target).first().cloned();
        let target_entity = target.as_ref().and_then(|target| monster.game.entity(target).cloned());
        monster.monster.path = target_entity
            .as_ref()
            .map(|entity| entity.targetname.clone())
            .unwrap_or_default();
        let id = monster.id.clone();
        monster.game.update_entity(&id, |entity| {
            entity.references.insert("goalentity".to_string(), target.clone());
            entity.references.insert("movetarget".to_string(), target.clone());
        })?;
        let destination = target
            .as_ref()
            .and_then(|target| monster.game.body(target).ok())
            .map(|body| body.origin)
            .unwrap_or(ZERO);
        let origin = monster.origin()?;
        let yaw = yaw_for(vsub(destination, origin));
        monster.game.update_entity(&id, |entity| {
            entity.ideal_yaw = yaw;
        })?;
        if target.is_some() {
            if corner_entity.delay != 0.0 {
                monster.monster.pause_until = monster.game.time + corner_entity.delay;
                let stand = monster.spec.stand;
                monster.play(stand)?;
            }
            return Ok(true);
        }
    }
    monster.monster.pause_until = monster.game.time + 999999.0;
    let stand = monster.spec.stand;
    monster.play(stand)?;
    Ok(true)
}

fn movetarget_touch_handler(game: &mut Q1EntityServices, corner: &ActorId, mover: &ActorId) -> Result<bool, Q1Error> {
    let owned = match game.host.actors.resolve_owned(mover) {
        Some(owned) => owned,
        None => return Ok(false),
    };
    let mut runtime = hipnotic_runtime();
    if runtime.contains(&owned) {
        let mut monster = match runtime.require(game, mover) {
            Ok(monster) => monster,
            Err(_) => return Ok(false),
        };
        let corner = corner.clone();
        let handled = movetarget_mission(&mut monster, &corner);
        monster.finish();
        store_hipnotic_runtime(runtime);
        return Ok(handled);
    }
    let Ok(mut monster) = BaseMonster::load(game, mover) else {
        return Ok(false);
    };
    let result = movetarget_base(&mut monster, corner);
    monster.finish()?;
    result
}

/// Register Hipnotic path following (`registerHipnoticPaths`).
pub fn register_hipnotic_paths(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        "hipnotic:t_followtarget",
        Q1CallbackHandlers {
            touch: Some(follow_touch_handler),
            ..Default::default()
        },
    )?;
    for classname in ["path_follow", "path_follow2"] {
        game.register_spawn(classname, path_follow_spawn)?;
    }
    game.register_path_touch("hipnotic:t_movetarget", movetarget_touch_handler)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::missionpacks::types::test_game;

    #[test]
    fn path_spawns_register() {
        let mut game = test_game();
        register_hipnotic_paths(&mut game).expect("register");
        let id = game.create("path_follow", None, None).expect("create");
        path_follow_spawn(&mut game, &id).expect("spawn");
        let entity = game.entity(&id).expect("entity").clone();
        assert_eq!(entity.solid, Q1Solid::Trigger);
        assert!(entity.touch.is_some());
        assert_eq!(entity.model, "");
        let id = game.create("path_follow2", None, None).expect("create");
        path_follow_spawn(&mut game, &id).expect("spawn");
        let body = game.body(&id).expect("body");
        assert_eq!(body.bounds.max.x, 8.0);
    }

    #[test]
    fn follow_ignores_non_monsters() {
        let mut game = test_game();
        register_hipnotic_paths(&mut game).expect("register");
        let trigger = game.create("path_follow", None, None).expect("trigger");
        let other = game.create("item_shells", None, None).expect("other");
        follow_touch_handler(&mut game, &trigger, &other, None, None).expect("touch");
        assert!(!movetarget_touch_handler(&mut game, &trigger, &other).expect("movetarget"));
    }
}
