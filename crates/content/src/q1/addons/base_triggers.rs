//! Q1 addon base triggers (`src/content/q1/addons/base-triggers.ts`).
//!
//! `quakec_mg1/triggers.qc` and `quakec_mg3/triggers.qc`.
//! GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::q1::addons::context::{
    addon_program, init_trigger, removed_for_runes, removed_outside_coop, require_entity, Q1AddonContext,
    Q1AddonProgram,
};
use crate::q1::base::map_entities::spawn_remaining_map_actor;
use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::TouchSurface;
use crate::q1::foundation::spawns::spawn_map_actor;
use crate::q1::foundation::types::Q1Solid;
use crate::q1::{q1_error, Q1Error};

fn prefix(game: &Q1EntityServices) -> Result<String, Q1Error> {
    Ok(format!("{}:trigger:", addon_program(game)?.as_str()))
}

/// Honey triggers with the grounded flag observe only grounded
/// activators (`grounded`).
fn grounded(game: &Q1EntityServices, id: &ActorId, other: Option<&ActorId>) -> Result<bool, Q1Error> {
    let entity = require_entity(game, id)?;
    if addon_program(game)? != Q1AddonProgram::Mg3 || (entity.spawnflags & 64) == 0 {
        return Ok(true);
    }
    let Some(other) = other else {
        return Ok(false);
    };
    Ok(game.host.bodies.read(other).is_some_and(|body| body.ground.is_some()))
}

fn multi_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    normal: Option<Vec3>,
    surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    if !grounded(game, id, Some(other))? {
        return Ok(());
    }
    let touch = game.named.touch_handler("multi_touch")?;
    let surface = surface.cloned();
    touch(game, id, other, normal, surface.as_ref())
}

fn multi_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    if !grounded(game, id, other)? {
        return Ok(());
    }
    let use_callback = game.named.use_handler("multi_use")?;
    use_callback(game, id, other, activator)
}

fn multi_killed(game: &mut Q1EntityServices, id: &ActorId, attacker: Option<&ActorId>) -> Result<(), Q1Error> {
    if !grounded(game, id, attacker)? {
        return Ok(());
    }
    let die = game.named.die_handler("multi_killed")?;
    die(game, id, attacker)
}

fn multiple(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if removed_outside_coop(game, id, true)? || removed_for_runes(game, id)? {
        return Ok(());
    }
    game.update_entity(id, |entity| {
        entity
            .fields
            .insert(String::from("netname"), String::from("trigger_multiple"));
    })?;
    if (require_entity(game, id)?.spawnflags & 2) != 0 {
        game.update_entity(id, |entity| entity.spawnflags &= !2)?;
        let use_callback = game.named.use_callback(&format!("{}activate_multi", prefix(game)?))?;
        return game.update_entity(id, |entity| entity.use_callback = Some(use_callback));
    }
    if require_entity(game, id)?.wait == 0.0 {
        game.update_entity(id, |entity| entity.wait = 0.2)?;
    }
    init_trigger(game, id)?;
    if !game.is_live(id) {
        return Ok(());
    }
    let use_callback = game.named.use_callback(&format!("{}multi_use", prefix(game)?))?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_callback))?;
    let entity = require_entity(game, id)?.clone();
    if entity.max_health != 0.0 {
        if (entity.spawnflags & 1) != 0 {
            return Err(q1_error("health and notouch don't make sense"));
        }
        game.set_damageable(id, true)?;
        let die = game.named.die(&format!("{}multi_killed", prefix(game)?))?;
        game.update_entity(id, |entity| {
            entity.solid = Q1Solid::Bbox;
            entity.die = Some(die);
        })?;
    } else if (entity.spawnflags & 1) == 0 {
        let touch = game.named.touch(&format!("{}multi_touch", prefix(game)?))?;
        game.update_entity(id, |entity| entity.touch = Some(touch))?;
    }
    Ok(())
}

fn activate_multi(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    multiple(game, id)
}

fn spawn_multi_trigger(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let program = addon_program(game)?;
    if removed_outside_coop(game, id, true)? || removed_for_runes(game, id)? {
        return Ok(());
    }
    let classname = require_entity(game, id)?.classname.clone();
    if classname != "trigger_multiple" {
        game.update_entity(id, |entity| entity.wait = -1.0)?;
    }
    if classname == "trigger_secret" {
        game.total_secrets += 1;
        let entity = require_entity(game, id)?.clone();
        if program != Q1AddonProgram::Mg3 || (entity.spawnflags & 128) == 0 {
            game.update_entity(id, |entity| {
                if entity.message.is_empty() {
                    entity.message = String::from("$qc_found_secret");
                }
            })?;
        }
        game.update_entity(id, |entity| {
            if entity.sounds == 0 {
                entity.sounds = 1;
            }
        })?;
    }
    multiple(game, id)?;
    if classname == "trigger_once" {
        game.update_entity(id, |entity| {
            entity
                .fields
                .insert(String::from("netname"), String::from("trigger_once"));
        })?;
    }
    Ok(())
}

fn spawn_teleport_trigger(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let relay = require_entity(game, id)?.classname == "trigger_relay";
    if removed_outside_coop(game, id, relay)? || removed_for_runes(game, id)? {
        return Ok(());
    }
    spawn_map_actor(game, id)
}

fn spawn_remaining_trigger(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if removed_outside_coop(game, id, false)? || removed_for_runes(game, id)? {
        return Ok(());
    }
    spawn_remaining_map_actor(game, id)
}

/// Register addon base triggers (`registerAddonBaseTriggers`).
pub fn register_addon_base_triggers(context: &Q1AddonContext, game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    let prefix = format!("{}:trigger:", context.program().as_str());
    game.named.register(
        &format!("{prefix}multi_touch"),
        Q1CallbackHandlers {
            touch: Some(multi_touch),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}multi_use"),
        Q1CallbackHandlers {
            use_callback: Some(multi_use),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}multi_killed"),
        Q1CallbackHandlers {
            die: Some(multi_killed),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}activate_multi"),
        Q1CallbackHandlers {
            use_callback: Some(activate_multi),
            ..Default::default()
        },
    )?;
    for classname in ["trigger_multiple", "trigger_once", "trigger_secret"] {
        game.register_spawn(classname, spawn_multi_trigger)?;
    }
    for classname in ["trigger_teleport", "trigger_relay"] {
        game.register_spawn(classname, spawn_teleport_trigger)?;
    }
    for classname in ["trigger_onlyregistered", "trigger_monsterjump", "trigger_setskill"] {
        game.replace_spawn(classname, spawn_remaining_trigger)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::register_test_addons;
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::foundation::types::Q1MoveType;
    use crate::q1::missionpacks::types::test_game;

    fn setup(game: &mut Q1EntityServices, program: Q1AddonProgram) -> (Q1BaseGuard, Q1AddonContext) {
        let guard = Q1BaseGuard::register(game, Q1BaseOptions::default()).expect("base");
        let context = register_test_addons(game, program);
        register_addon_base_triggers(&context, game).expect("triggers");
        (guard, context)
    }

    #[test]
    fn multiple_arms_touch_and_use() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg1);
        let trigger = game.create("trigger_multiple", None, None).expect("trigger");
        spawn_multi_trigger(&mut game, &trigger).expect("spawn");
        let entity = require_entity(&game, &trigger).expect("entity");
        assert_eq!(entity.wait, 0.2);
        assert_eq!(entity.solid, Q1Solid::Trigger);
        assert_eq!(entity.movement, Q1MoveType::None);
        assert_eq!(entity.touch.as_deref(), Some("mg1:trigger:multi_touch"));
        assert_eq!(entity.use_callback.as_deref(), Some("mg1:trigger:multi_use"));

        let delayed = game.create("trigger_multiple", None, None).expect("delayed");
        game.update_entity(&delayed, |entity| entity.spawnflags = 2)
            .expect("flags");
        spawn_multi_trigger(&mut game, &delayed).expect("spawn");
        let entity = require_entity(&game, &delayed).expect("entity");
        assert_eq!(entity.spawnflags & 2, 0);
        assert_eq!(entity.use_callback.as_deref(), Some("mg1:trigger:activate_multi"));
        assert_eq!(entity.touch, None);
    }

    #[test]
    fn once_and_secret_tune_spawns() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg1);
        let once = game.create("trigger_once", None, None).expect("once");
        spawn_multi_trigger(&mut game, &once).expect("spawn");
        let entity = require_entity(&game, &once).expect("entity");
        assert_eq!(entity.wait, -1.0);
        assert_eq!(entity.text("netname"), "trigger_once");

        let secrets = game.total_secrets;
        let secret = game.create("trigger_secret", None, None).expect("secret");
        spawn_multi_trigger(&mut game, &secret).expect("spawn");
        assert_eq!(game.total_secrets, secrets + 1);
        let entity = require_entity(&game, &secret).expect("entity");
        assert_eq!(entity.message, "$qc_found_secret");
        assert_eq!(entity.sounds, 1);
    }

    #[test]
    fn health_and_notouch_is_rejected() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg1);
        let trigger = game.create("trigger_multiple", None, None).expect("trigger");
        game.update_entity(&trigger, |entity| {
            entity.spawnflags = 1;
            entity.max_health = 10.0;
        })
        .expect("setup");
        assert!(spawn_multi_trigger(&mut game, &trigger).is_err());

        let killable = game.create("trigger_multiple", None, None).expect("killable");
        game.update_entity(&killable, |entity| entity.max_health = 10.0)
            .expect("setup");
        spawn_multi_trigger(&mut game, &killable).expect("spawn");
        let entity = require_entity(&game, &killable).expect("entity");
        assert_eq!(entity.solid, Q1Solid::Bbox);
        assert_eq!(entity.die.as_deref(), Some("mg1:trigger:multi_killed"));
        assert!(game.is_damageable(&killable));
    }

    #[test]
    fn teleport_and_remaining_triggers_delegate() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg1);
        let teleport = game.create("trigger_teleport", None, None).expect("teleport");
        game.update_entity(&teleport, |entity| entity.target = String::from("dest"))
            .expect("target");
        spawn_teleport_trigger(&mut game, &teleport).expect("spawn");
        assert!(game.entity_ref(&teleport).is_some());
        let relay = game.create("trigger_relay", None, None).expect("relay");
        spawn_teleport_trigger(&mut game, &relay).expect("spawn");
        assert!(require_entity(&game, &relay).expect("entity").use_callback.is_some());
        let jump = game.create("trigger_monsterjump", None, None).expect("jump");
        spawn_remaining_trigger(&mut game, &jump).expect("spawn");
        assert!(game.entity_ref(&jump).is_some());
    }

    #[test]
    fn honey_grounded_gates_observe_ground() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg3);
        let trigger = game.create("trigger_multiple", None, None).expect("trigger");
        game.update_entity(&trigger, |entity| entity.spawnflags = 64)
            .expect("flags");
        spawn_multi_trigger(&mut game, &trigger).expect("spawn");
        let other = game.create("info_null", None, None).expect("other");
        assert_eq!(grounded(&game, &trigger, Some(&other)), Ok(false));
        assert_eq!(grounded(&game, &trigger, None), Ok(false));
        multi_touch(&mut game, &trigger, &other, None, None).expect("touch");
        assert!(game.entity_ref(&trigger).is_some());
    }
}
