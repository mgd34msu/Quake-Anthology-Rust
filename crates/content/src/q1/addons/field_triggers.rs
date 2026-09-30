//! Q1 addon field triggers (`src/content/q1/addons/field-triggers.ts`).
//!
//! `quakec_mg1/triggers.qc` and `quakec_mg3/triggers.qc`.
//! GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::q1::addons::context::{
    addon_frame_time, addon_is_monster, addon_player_number, addon_program, init_trigger, require_entity,
    set_addon_number, set_addon_player_number, set_addon_vector, Q1AddonContext, Q1AddonProgram,
};
use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::TouchSurface;
use crate::q1::foundation::types::{dot, length, normalize, vadd, vscale, vsub, Q1Solid, Q1SoundChannel};
use crate::q1::Q1Error;

fn prefix(game: &Q1EntityServices) -> Result<String, Q1Error> {
    Ok(format!("{}:field:", addon_program(game)?.as_str()))
}

fn hurt_on(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| entity.solid = Q1Solid::Trigger)?;
    game.cancel(id);
    Ok(())
}

fn hurt_toggle(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let state = require_entity(game, id)?.number("state");
    set_addon_number(game, id, "state", 1.0 - state)
}

fn hurt_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let entity = require_entity(game, id)?.clone();
    if entity.number("state") != 0.0
        || !game
            .host
            .combat
            .read(other)
            .is_some_and(|combat| combat.can_take_damage)
        || ((entity.spawnflags & 8) != 0 && !addon_is_monster(game, other)?)
    {
        return Ok(());
    }
    game.update_entity(id, |entity| entity.solid = Q1Solid::None)?;
    game.damage(
        other,
        Some(id),
        Some(id),
        entity.damage,
        &crate::q1::foundation::entity_services::Q1DamageParams::default(),
    );
    let callback = game.named.action(&format!("{}hurt_on", prefix(game)?))?;
    game.schedule(id, entity.wait, &callback)
}

fn spawn_trigger_hurt(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    init_trigger(game, id)?;
    if !game.is_live(id) {
        return Ok(());
    }
    game.update_entity(id, |entity| {
        entity
            .fields
            .insert(String::from("netname"), String::from("trigger_hurt"));
        if entity.damage == 0.0 {
            entity.damage = 5.0;
        }
        if entity.wait == 0.0 {
            entity.wait = 1.0;
        }
    })?;
    if (require_entity(game, id)?.spawnflags & 1) != 0 {
        set_addon_number(game, id, "state", 1.0)?;
    }
    let touch = game.named.touch(&format!("{}hurt", prefix(game)?))?;
    let use_callback = game.named.use_callback(&format!("{}hurt", prefix(game)?))?;
    game.update_entity(id, |entity| {
        entity.touch = Some(touch);
        entity.use_callback = Some(use_callback);
    })
}

fn push_toggle(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    if require_entity(game, id)?.solid == Q1Solid::Trigger {
        game.update_entity(id, |entity| entity.solid = Q1Solid::None)?;
    } else {
        game.update_entity(id, |entity| entity.solid = Q1Solid::Trigger)?;
        game.force_retouch = 1;
    }
    Ok(())
}

fn push_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let program = addon_program(game)?;
    let actor = match game.host.actors.resolve_owned(other) {
        Some(actor) => actor,
        None => return Ok(()),
    };
    let body = match game.host.bodies.read(other) {
        Some(body) => body,
        None => return Ok(()),
    };
    let entity = require_entity(game, id)?.clone();
    let source = game.entity_ref(other).cloned();
    let classname = game.host.classname(other);
    let player = game.is_player(other);
    let delta = vscale(entity.movedir, entity.speed * 10.0);
    let velocity = if (entity.spawnflags & 2) != 0 {
        vadd(body.velocity, vscale(delta, addon_frame_time(game)?))
    } else {
        delta
    };
    if program != Q1AddonProgram::Mg3 && game.entities.values().any(|target| target.classname == "horde_manager") {
        if source.is_some() {
            game.update_entity(other, |entity| entity.spawnflags &= !4)?;
        }
        if !player {
            if classname == "item_artifact_invulnerability" || classname == "item_artifact_super_damage" {
                let mut updated = body.clone();
                updated.velocity = velocity;
                game.host.bodies.write(&actor, &updated)?;
            }
            return Ok(());
        }
    }
    if game.health(other) > 0.0 || classname == "grenade" {
        let sheltered = match &source {
            None => addon_player_number(game, other, "in_shelter")? != 0.0,
            Some(source) => (source.movement_flags & 32768) != 0,
        };
        if sheltered {
            return Ok(());
        }
        let jump = program == Q1AddonProgram::Mg3 && (entity.spawnflags & 16) != 0;
        if jump && body.ground.is_none() {
            return Ok(());
        }
        let monster = addon_is_monster(game, other)?;
        let mut updated = body.clone();
        updated.velocity = velocity;
        if monster {
            updated.ground = None;
        }
        game.host.bodies.write(&actor, &updated)?;
        if monster && source.is_some() {
            game.update_entity(other, |entity| entity.movement_flags &= !512)?;
        }
        if player && addon_player_number(game, other, "fly_sound")? < game.time {
            set_addon_player_number(game, other, "fly_sound", game.time + if jump { 0.5 } else { 1.5 })?;
            game.sound(
                actor.id(),
                if jump {
                    "weapons/sgun1.wav"
                } else if program == Q1AddonProgram::Mg3 && (entity.spawnflags & 8) != 0 {
                    "player/inh2o.wav"
                } else {
                    "ambience/windfly.wav"
                },
                Q1SoundChannel::Auto,
                1.0,
                1.0,
            )?;
        }
    }
    if (entity.spawnflags & 1) != 0 {
        game.remove(id)?;
    }
    Ok(())
}

fn spawn_trigger_push(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let program = addon_program(game)?;
    if program == Q1AddonProgram::Mg3 {
        let angles = game.body(id)?.angles;
        let direction = require_entity(game, id)?.vector("movedir");
        if length(angles) == 0.0 && length(direction) == 0.0 {
            set_addon_vector(game, id, "movedir", Vec3 { x: 1.0, y: 0.0, z: 0.0 })?;
        } else if length(direction) != 0.0 {
            set_addon_vector(game, id, "movedir", normalize(direction))?;
        }
    }
    init_trigger(game, id)?;
    if !game.is_live(id) {
        return Ok(());
    }
    if program == Q1AddonProgram::Mg3 && require_entity(game, id)?.fields.contains_key("movedir") {
        let movedir = require_entity(game, id)?.vector("movedir");
        game.update_entity(id, |entity| entity.movedir = movedir)?;
    }
    game.update_entity(id, |entity| {
        entity
            .fields
            .insert(String::from("netname"), String::from("trigger_push"));
        if entity.speed == 0.0 {
            entity.speed = 1000.0;
        }
    })?;
    let touch = game.named.touch(&format!("{}push", prefix(game)?))?;
    let use_callback = game.named.use_callback(&format!("{}push", prefix(game)?))?;
    game.update_entity(id, |entity| {
        entity.touch = Some(touch);
        entity.use_callback = Some(use_callback);
        if (entity.spawnflags & 4) != 0 {
            entity.solid = Q1Solid::None;
        }
    })
}

fn shelter_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    if game.health(other) <= 0.0 && game.host.classname(other) != "grenade" {
        return Ok(());
    }
    let body = match game.host.bodies.read(other) {
        Some(body) => body,
        None => return Ok(()),
    };
    let entity = require_entity(game, id)?.clone();
    let sheltered = dot(vsub(body.origin, entity.pos1), entity.pos2) >= 0.0;
    if game.entity_ref(other).is_some() {
        game.update_entity(other, |entity| {
            if sheltered {
                entity.movement_flags |= 32768;
            } else {
                entity.movement_flags &= !32768;
            }
        })?;
    } else {
        set_addon_player_number(game, other, "in_shelter", if sheltered { 1.0 } else { 0.0 })?;
    }
    Ok(())
}

fn spawn_trigger_shelter_portal(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    init_trigger(game, id)?;
    if !game.is_live(id) {
        return Ok(());
    }
    let bounds = game.body(id)?.bounds;
    let size = vsub(bounds.max, bounds.min);
    let mut pos2 = if size.x < size.y {
        if size.x < size.z {
            Vec3 { x: 1.0, y: 0.0, z: 0.0 }
        } else {
            Vec3 { x: 0.0, y: 0.0, z: 1.0 }
        }
    } else if size.y < size.z {
        Vec3 { x: 0.0, y: 1.0, z: 0.0 }
    } else {
        Vec3 { x: 0.0, y: 0.0, z: 1.0 }
    };
    if (require_entity(game, id)?.spawnflags & 1) != 0 {
        pos2 = vscale(pos2, -1.0);
    }
    let pos1 = vadd(bounds.min, vscale(size, 0.5));
    game.update_entity(id, |entity| {
        entity.pos1 = pos1;
        entity.pos2 = pos2;
    })?;
    let touch = game.named.touch(&format!("{}shelter", prefix(game)?))?;
    game.update_entity(id, |entity| entity.touch = Some(touch))
}

/// Register addon field triggers (`registerAddonFieldTriggers`).
pub fn register_addon_field_triggers(context: &Q1AddonContext, game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    let prefix = format!("{}:field:", context.program().as_str());
    game.named.register(
        &format!("{prefix}hurt_on"),
        Q1CallbackHandlers {
            action: Some(hurt_on),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}hurt"),
        Q1CallbackHandlers {
            use_callback: Some(hurt_toggle),
            touch: Some(hurt_touch),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_hurt", spawn_trigger_hurt)?;
    game.named.register(
        &format!("{prefix}push"),
        Q1CallbackHandlers {
            use_callback: Some(push_toggle),
            touch: Some(push_touch),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_push", spawn_trigger_push)?;
    game.named.register(
        &format!("{prefix}shelter"),
        Q1CallbackHandlers {
            touch: Some(shelter_touch),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_shelter_portal", spawn_trigger_shelter_portal)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::{attach_test_player, register_test_addons};
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::foundation::types::Q1MoveType;
    use crate::q1::missionpacks::types::test_game;

    fn setup(game: &mut Q1EntityServices, program: Q1AddonProgram) -> (Q1BaseGuard, Q1AddonContext) {
        let guard = Q1BaseGuard::register(game, Q1BaseOptions::default()).expect("base");
        let context = register_test_addons(game, program);
        register_addon_field_triggers(&context, game).expect("triggers");
        (guard, context)
    }

    #[test]
    fn hurt_arms_with_defaults_and_toggles() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg1);
        let hurt = game.create("trigger_hurt", None, None).expect("hurt");
        spawn_trigger_hurt(&mut game, &hurt).expect("spawn");
        let entity = require_entity(&game, &hurt).expect("entity");
        assert_eq!(entity.damage, 5.0);
        assert_eq!(entity.wait, 1.0);
        assert_eq!(entity.touch.as_deref(), Some("mg1:field:hurt"));
        hurt_toggle(&mut game, &hurt, None, None).expect("toggle");
        assert_eq!(require_entity(&game, &hurt).expect("entity").number("state"), 1.0);
        hurt_toggle(&mut game, &hurt, None, None).expect("toggle");
        assert_eq!(require_entity(&game, &hurt).expect("entity").number("state"), 0.0);
    }

    #[test]
    fn hurt_damages_and_rearms() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg1);
        let hurt = game.create("trigger_hurt", None, None).expect("hurt");
        spawn_trigger_hurt(&mut game, &hurt).expect("spawn");
        let player = attach_test_player(&mut game);
        hurt_touch(&mut game, &hurt, &player, None, None).expect("touch");
        let entity = require_entity(&game, &hurt).expect("entity");
        assert_eq!(entity.solid, Q1Solid::None);
        assert_eq!(entity.think.as_deref(), Some("mg1:field:hurt_on"));
        hurt_on(&mut game, &hurt).expect("rearm");
        assert_eq!(require_entity(&game, &hurt).expect("entity").solid, Q1Solid::Trigger);
    }

    #[test]
    fn push_defaults_and_toggles() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg1);
        let push = game.create("trigger_push", None, None).expect("push");
        spawn_trigger_push(&mut game, &push).expect("spawn");
        let entity = require_entity(&game, &push).expect("entity");
        assert_eq!(entity.speed, 1000.0);
        assert_eq!(entity.movement, Q1MoveType::None);
        push_toggle(&mut game, &push, None, None).expect("toggle");
        assert_eq!(require_entity(&game, &push).expect("entity").solid, Q1Solid::None);
        push_toggle(&mut game, &push, None, None).expect("toggle");
        assert_eq!(require_entity(&game, &push).expect("entity").solid, Q1Solid::Trigger);
        assert_eq!(game.force_retouch, 1);
    }

    #[test]
    fn honey_push_normalizes_movedir() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg3);
        let push = game.create("trigger_push", None, None).expect("push");
        spawn_trigger_push(&mut game, &push).expect("spawn");
        assert_eq!(
            require_entity(&game, &push).expect("entity").vector("movedir"),
            Vec3 { x: 1.0, y: 0.0, z: 0.0 }
        );
    }

    #[test]
    fn shelter_portal_marks_shelter_side() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg1);
        let portal = game.create("trigger_shelter_portal", None, None).expect("portal");
        spawn_trigger_shelter_portal(&mut game, &portal).expect("spawn");
        assert_eq!(
            require_entity(&game, &portal).expect("entity").touch.as_deref(),
            Some("mg1:field:shelter")
        );
        let player = attach_test_player(&mut game);
        shelter_touch(&mut game, &portal, &player, None, None).expect("touch");
        assert_ne!(
            require_entity(&game, &player).expect("player").movement_flags & 32768,
            0
        );
    }
}
