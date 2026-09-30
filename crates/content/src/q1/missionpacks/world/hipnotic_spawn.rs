//! Hipnotic monster spawn molds (`src/content/q1/missionpacks/world/hipnotic-spawn.ts`).
//!
//! hipspawn.qc SUB_CopyEntity master molds.

use qa_core::identity::ActorId;

use crate::bsp::Q1Entity;
use crate::q1::foundation::callbacks::{Q1CallbackHandlers, callback_name};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::BodyPatch;
use crate::q1::foundation::spawns::spawn_teleport_fog;
use crate::q1::foundation::types::{Q1MoveType, Q1Solid};
use crate::q1::{Q1Error, q1_error};

use super::common::{later, number, vector};
use super::with_missionpack_hooks;

/// Parse a saved solidity value (`solid`).
fn solid(value: &str) -> Result<Q1Solid, Q1Error> {
    Q1Solid::parse(value).map_err(|_| q1_error(format!("Invalid func_spawn saved solid {value}")))
}

/// Spawn a hidden template mold, saving its live state (`template`).
fn template(
    game: &mut Q1EntityServices,
    mold: &ActorId,
    classname: &str,
) -> Result<ActorId, Q1Error> {
    let properties: Vec<(String, String)> = game
        .entity(mold)
        .map(|mold| {
            mold.fields
                .iter()
                .filter(|(key, _)| key.as_str() != "classname")
                .map(|(key, value)| (key.clone(), value.clone()))
                .chain(std::iter::once((
                    "classname".to_string(),
                    classname.to_string(),
                )))
                .collect()
        })
        .unwrap_or_default();
    let source = Q1Entity { properties };
    let entity = game.create(classname, Some(&source), None)?;
    let mold_body = game.body(mold)?;
    game.set_body(
        &entity,
        &BodyPatch {
            origin: Some(mold_body.origin),
            angles: Some(mold_body.angles),
            velocity: Some(mold_body.velocity),
            bounds: Some(mold_body.bounds),
            ground: Some(mold_body.ground.clone()),
        },
    )?;
    game.spawn_entity(&entity, Some(0))?;
    if !game.is_live(&entity) {
        return Err(q1_error(format!(
            "func_spawn template {classname} removed itself"
        )));
    }
    let body = game.body(&entity)?;
    let (model, solid, think) = game
        .entity(&entity)
        .map(|entity| {
            (
                entity.model.clone(),
                entity.solid,
                callback_name(entity.think.as_ref()),
            )
        })
        .unwrap_or_default();
    game.update_entity(&entity, |entity| {
        entity.fields.insert("spawnmodel".to_string(), model);
        entity
            .fields
            .insert("spawnsolidtype".to_string(), solid.as_str().to_string());
        match think {
            Some(think) => {
                entity.fields.insert("spawnthink".to_string(), think);
            }
            None => {
                entity.fields.remove("spawnthink");
            }
        }
        vector(entity, "spawnmins", body.bounds.min);
        vector(entity, "spawnmaxs", body.bounds.max);
        entity.model.clear();
        entity.solid = Q1Solid::None;
    })?;
    later(game, &entity, 1.0, "hip:spawn_think")?;
    game.link(&entity)?;
    Ok(entity)
}

/// Release (or clone) the mold master on use.
fn spawn_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let master = game
        .entity(id)
        .and_then(|entity| entity.references.get("spawnmaster").cloned().flatten())
        .filter(|master| game.entity(master).is_some())
        .ok_or_else(|| q1_error("func_spawn lost its initialized master"))?;
    let charmer = with_missionpack_hooks(game, |_, hooks| {
        Ok(hooks.charmer.as_ref().map(|charmer| charmer()))
    })?;
    let spawn_multi = game
        .entity(id)
        .map(|entity| entity.number("spawnmulti"))
        .unwrap_or(0.0);
    let entity = if spawn_multi == 1.0 || charmer.is_some() {
        game.clone_entity(&master)?
    } else {
        master.clone()
    };
    let (model, solid_text, think, mins, maxs) = game
        .entity(&entity)
        .map(|entity| {
            (
                entity.text("spawnmodel"),
                entity.text("spawnsolidtype"),
                entity.text("spawnthink"),
                entity.vector("spawnmins"),
                entity.vector("spawnmaxs"),
            )
        })
        .unwrap_or_default();
    let think = if think.is_empty() {
        None
    } else {
        Some(game.named.action(&think)?)
    };
    let solid_value = solid(&solid_text)?;
    game.update_entity(&entity, |entity| {
        entity.model = model;
        entity.solid = solid_value;
        entity.think = think;
    })?;
    game.set_bounds(
        &entity,
        qa_core::math::Bounds {
            min: mins,
            max: maxs,
        },
    )?;
    game.link(&entity)?;
    let origin = game.body(&entity)?.origin;
    if game
        .entity(id)
        .map(|entity| entity.number("spawnsilent"))
        .unwrap_or(0.0)
        == 0.0
    {
        spawn_teleport_fog(game, origin)?;
    }
    if let Some(charmer) = charmer.clone() {
        with_missionpack_hooks(game, |game, hooks| {
            let charm = hooks.charm.as_ref().ok_or_else(|| {
                q1_error("Horn func_spawn requires the selected monster charm implementation")
            })?;
            charm(game, &entity, &charmer)
        })?;
    }
    let movement_flags = game
        .entity(&entity)
        .map(|entity| entity.movement_flags)
        .unwrap_or(0);
    if movement_flags & 32 != 0 {
        if spawn_multi != 0.0 && charmer.is_none() {
            game.total_monsters += 1;
        }
        if charmer.is_some() {
            game.update_entity(&entity, |entity| entity.effects |= 8)?;
        }
    }
    if spawn_multi == 0.0 && charmer.is_none() {
        return game.remove(id);
    }
    Ok(())
}

/// Keep a mold master parked.
fn spawn_think(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    later(game, id, 1.0, "hip:spawn_think")
}

/// Spawn a `func_spawn` or `func_spawn_small` mold.
fn spawn_func_spawn(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let count = game.total_monsters;
    let spawn_function = game
        .entity(id)
        .map(|entity| entity.text("spawnfunction"))
        .unwrap_or_default();
    let master = if spawn_function.is_empty() {
        let chance = game.host.random();
        let dog = template(game, id, "monster_dog")?;
        let ogre = template(game, id, "monster_ogre")?;
        let demon = template(game, id, "monster_demon1")?;
        let zombie = template(game, id, "monster_zombie")?;
        let shambler = template(game, id, "monster_shambler")?;
        let master = if chance < 0.5 {
            dog
        } else if chance < 0.8 {
            ogre
        } else if chance < 0.92 {
            demon
        } else if chance < 0.97 {
            zombie
        } else {
            shambler
        };
        game.total_monsters = count + 1;
        master
    } else {
        let spawn_class = game
            .entity(id)
            .map(|entity| entity.text("spawnclassname"))
            .unwrap_or_default();
        if spawn_class.is_empty() {
            return Err(q1_error("No spawnclassname defined"));
        }
        let master = template(game, id, &spawn_class)?;
        if game
            .entity(id)
            .map(|entity| entity.number("spawnmulti"))
            .unwrap_or(0.0)
            != 0.0
        {
            game.total_monsters = count;
        }
        master
    };
    let use_name = game.named.use_callback("hip:spawn_use")?;
    game.update_entity(id, |entity| {
        entity.solid = Q1Solid::None;
        entity.movement = Q1MoveType::None;
        entity.model.clear();
        entity.use_callback = Some(use_name);
        entity
            .references
            .insert("spawnmaster".to_string(), Some(master));
    })
}

/// Register Hipnotic spawn molds (`registerHipnoticSpawn`).
pub fn register_hipnotic_spawn(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        "hip:spawn_think",
        Q1CallbackHandlers {
            action: Some(spawn_think),
            ..Default::default()
        },
    )?;
    game.named.register(
        "hip:spawn_use",
        Q1CallbackHandlers {
            use_callback: Some(spawn_use),
            ..Default::default()
        },
    )?;
    game.register_spawn("func_spawn", spawn_func_spawn)?;
    game.register_spawn("func_spawn_small", spawn_func_spawn)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::missionpacks::types::test_game;

    #[test]
    fn mold_spawns_master_and_parks_it() {
        let mut game = test_game();
        register_hipnotic_spawn(&mut game).expect("register");
        let id = game.create("func_spawn", None, None).expect("mold");
        game.update_entity(&id, |entity| {
            entity
                .fields
                .insert("spawnfunction".to_string(), "test".to_string());
            entity
                .fields
                .insert("spawnclassname".to_string(), "mold_class".to_string());
        })
        .expect("fields");
        game.spawn_entity(&id, None).expect("spawn");
        let mold = game.entity(&id).cloned().expect("mold");
        assert_eq!(mold.use_callback.as_deref(), Some("hip:spawn_use"));
        let master = mold
            .references
            .get("spawnmaster")
            .cloned()
            .flatten()
            .expect("master");
        let master_entity = game.entity(&master).cloned().expect("master entity");
        assert_eq!(master_entity.think.as_deref(), Some("hip:spawn_think"));
        assert!(master_entity.model.is_empty());
    }

    #[test]
    fn spawn_use_releases_master_and_removes_trigger() {
        let mut game = test_game();
        register_hipnotic_spawn(&mut game).expect("register");
        let id = game.create("func_spawn", None, None).expect("mold");
        game.update_entity(&id, |entity| {
            entity
                .fields
                .insert("spawnfunction".to_string(), "test".to_string());
            entity
                .fields
                .insert("spawnclassname".to_string(), "mold_class".to_string());
        })
        .expect("fields");
        game.spawn_entity(&id, None).expect("spawn");
        let master = game
            .entity(&id)
            .and_then(|entity| entity.references.get("spawnmaster").cloned().flatten())
            .expect("master");
        game.invoke_use(&id, "hip:spawn_use", None, None)
            .expect("use");
        assert!(game.entity(&id).is_none());
        assert!(game.entity(&master).is_some());
    }

    #[test]
    fn spawn_think_reparks_master() {
        let mut game = test_game();
        register_hipnotic_spawn(&mut game).expect("register");
        let id = game.create("func_spawn", None, None).expect("mold");
        game.invoke_action(&id, "hip:spawn_think").expect("think");
        assert_eq!(
            game.entity(&id).expect("mold").think.as_deref(),
            Some("hip:spawn_think")
        );
    }
}
