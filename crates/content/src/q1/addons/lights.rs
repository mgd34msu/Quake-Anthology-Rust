//! Q1 addon lights (`src/content/q1/addons/lights.ts`).
//!
//! `quakec_mg1/lights.qc` and `quakec_mg3/lights.qc`.
//! Copyright (C) 1996-2026 id Software LLC. GPL-2.0-or-later.

use qa_core::identity::ActorId;

use crate::q1::addons::context::{
    add_frame_tick, addon_alpha, addon_frame_time, addon_program, fround, remove_frame_tick, require_entity,
    set_addon_number, Q1AddonContext,
};
use crate::q1::base::map_entities::make_static;
use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::BodyPatch;
use crate::q1::foundation::types::Q1Event;
use crate::q1::{q1_error, Q1Error};
use qa_core::math::Vec3;

/// Light-style pattern character for a brightness fraction
/// (`addonLightStyle`).
#[must_use]
pub fn addon_light_style(fraction: f64, full_range: bool) -> String {
    let maximum: i32 = if full_range { 25 } else { 12 };
    let step = if fraction <= 0.0 {
        0
    } else if fraction >= 1.0 {
        maximum
    } else {
        (fraction * f64::from(maximum + 1)).floor() as i32
    };
    char::from(97 + step as u8).to_string()
}

fn prefix(game: &Q1EntityServices) -> Result<String, Q1Error> {
    Ok(format!("{}:light:", addon_program(game)?.as_str()))
}

fn schedule_action(game: &mut Q1EntityServices, id: &ActorId, name: &str, delay: f64) -> Result<(), Q1Error> {
    let callback = game.named.action(name)?;
    game.schedule(id, delay, &callback)
}

fn ramp_tick(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let entity = require_entity(game, id)?.clone();
    let owner = entity.owner.clone().filter(|owner| game.entity_ref(owner).is_some());
    let owner = owner.ok_or_else(|| q1_error("target_lightramp lost its light"))?;
    let state = entity.number("ramp.state");
    let delta = fround(addon_frame_time(game)? * entity.delay);
    let fraction = fround(
        entity.number("cnt")
            + if state == 3.0 {
                -delta
            } else if state == 1.0 {
                delta
            } else {
                0.0
            },
    );
    let style = require_entity(game, &owner)?.number("style") as i32;
    game.host.emit(Q1Event::Lightstyle {
        style,
        pattern: addon_light_style(fraction, (entity.spawnflags & 1) != 0),
    });
    set_addon_number(game, id, "cnt", fraction)?;
    if fraction >= 1.0 {
        set_addon_number(game, id, "cnt", 1.0)?;
        set_addon_number(game, id, "ramp.state", 2.0)?;
        remove_frame_tick(game, id)?;
    }
    if fraction <= 0.0 {
        set_addon_number(game, id, "cnt", 0.0)?;
        set_addon_number(game, id, "ramp.state", 0.0)?;
        remove_frame_tick(game, id)?;
    }
    Ok(())
}

fn ramp_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let state = require_entity(game, id)?.number("ramp.state");
    set_addon_number(
        game,
        id,
        "ramp.state",
        if state == 3.0 || state == 0.0 { 1.0 } else { 3.0 },
    )?;
    if state == 0.0 || state == 2.0 {
        add_frame_tick(game, id, &format!("{}ramp_tick", prefix(game)?))?;
    }
    Ok(())
}

fn ramp_init(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let entity = require_entity(game, id)?.clone();
    if entity.target.is_empty() || entity.targetname.is_empty() {
        return Err(q1_error("target_lightramp requires target and targetname"));
    }
    let owner = game
        .find(&entity.target)
        .first()
        .cloned()
        .ok_or_else(|| q1_error("target_lightramp has an unmatched target"))?;
    let off = (require_entity(game, &owner)?.spawnflags & 1) != 0;
    game.update_entity(id, |entity| entity.owner = Some(owner))?;
    set_addon_number(game, id, "ramp.state", if off { 0.0 } else { 2.0 })?;
    set_addon_number(game, id, "cnt", if off { 0.0 } else { 1.0 })?;
    let think = game.named.action("SUB_Null")?;
    let use_callback = game.named.use_callback(&format!("{}ramp_use", prefix(game)?))?;
    game.update_entity(id, |entity| {
        entity.think = Some(think);
        entity.use_callback = Some(use_callback);
    })
}

fn spawn_target_lightramp(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let delay = require_entity(game, id)?.delay;
    game.update_entity(id, |entity| {
        entity.delay = fround(1.0 / if delay == 0.0 { 1.0 } else { delay });
    })?;
    schedule_action(game, id, &format!("{}ramp_init", prefix(game)?), 0.1)
}

fn spawn_dynamiclight(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if game.options().coop && (require_entity(game, id)?.spawnflags & 1) != 0 {
        game.remove(id)?;
    }
    Ok(())
}

fn spawn_light(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let entity = require_entity(game, id)?.clone();
    if entity.targetname.is_empty() {
        return game.remove(id);
    }
    let style = entity.number("style");
    if style < 32.0 {
        return Ok(());
    }
    if (entity.spawnflags & 2) != 0 {
        game.host.emit(Q1Event::Lightstyle {
            style: style as i32,
            pattern: entity.targetname.clone(),
        });
        return game.remove(id);
    }
    game.host.emit(Q1Event::Lightstyle {
        style: style as i32,
        pattern: if (entity.spawnflags & 1) != 0 {
            String::from("a")
        } else {
            String::from("m")
        },
    });
    let use_callback = game.named.use_callback("light_use")?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_callback))
}

fn spawn_light_fluorospark(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if require_entity(game, id)?.number("style") == 0.0 {
        set_addon_number(game, id, "style", 10.0)?;
    }
    let origin = game.body(id)?.origin;
    game.host.emit(Q1Event::Ambient {
        origin,
        path: String::from("ambience/buzz1.wav"),
        volume: 0.5,
        attenuation: 3.0,
    });
    Ok(())
}

fn spawn_flame(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let entity = require_entity(game, id)?.clone();
    let model = if entity.classname == "light_torch_small_walltorch" {
        String::from("progs/flame.mdl")
    } else {
        String::from("progs/flame2.mdl")
    };
    game.precache_model(&model)?;
    game.update_entity(id, |entity| {
        entity.model = model;
        if entity.classname == "light_flame_large_yellow" {
            entity.frame = 1;
        }
    })?;
    if (require_entity(game, id)?.spawnflags & 4) != 0 {
        game.set_body(
            id,
            &BodyPatch {
                angles: Some(Vec3 {
                    x: 180.0,
                    y: 0.0,
                    z: 0.0,
                }),
                ..Default::default()
            },
        )?;
    }
    game.precache_sound("ambience/fire1.wav")?;
    let origin = game.body(id)?.origin;
    game.host.emit(Q1Event::Ambient {
        origin,
        path: String::from("ambience/fire1.wav"),
        volume: 0.5,
        attenuation: 3.0,
    });
    make_static(game, id)
}

fn spawn_light_flame_gas(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| entity.model = String::from("progs/flame3.mdl"))?;
    addon_alpha(game, id, 0.6)?;
    let second = game.create("gas_flame", None, None)?;
    game.update_entity(&second, |entity| {
        entity.model = String::from("progs/flame3.mdl");
        entity.frame = 1;
    })?;
    let origin = game.body(id)?.origin;
    game.set_origin(&second, origin)?;
    addon_alpha(game, &second, 0.4)?;
    game.link(&second)
}

fn spawn_light_candle(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let model = game.precache_model("progs/candle.mdl")?;
    game.update_entity(id, |entity| entity.model = model)?;
    make_static(game, id)
}

/// Register addon lights (`registerAddonLights`).
pub fn register_addon_lights(context: &Q1AddonContext, game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    let prefix = format!("{}:light:", context.program().as_str());
    game.register_spawn("dynamiclight", spawn_dynamiclight)?;
    game.named.register(
        &format!("{prefix}ramp_tick"),
        Q1CallbackHandlers {
            action: Some(ramp_tick),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}ramp_use"),
        Q1CallbackHandlers {
            use_callback: Some(ramp_use),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}ramp_init"),
        Q1CallbackHandlers {
            action: Some(ramp_init),
            ..Default::default()
        },
    )?;
    game.register_spawn("target_lightramp", spawn_target_lightramp)?;
    game.register_spawn("light", spawn_light)?;
    game.register_spawn("light_fluorospark", spawn_light_fluorospark)?;
    for classname in [
        "light_torch_small_walltorch",
        "light_flame_large_yellow",
        "light_flame_small_yellow",
        "light_flame_small_white",
    ] {
        game.replace_spawn(classname, spawn_flame)?;
    }
    game.register_spawn("light_flame_gas", spawn_light_flame_gas)?;
    game.register_spawn("light_candle", spawn_light_candle)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::{register_test_addons, Q1AddonProgram};
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::missionpacks::types::test_game;

    fn setup(game: &mut Q1EntityServices, program: Q1AddonProgram) -> (Q1BaseGuard, Q1AddonContext) {
        let guard = Q1BaseGuard::register(game, Q1BaseOptions::default()).expect("base");
        let context = register_test_addons(game, program);
        register_addon_lights(&context, game).expect("lights");
        (guard, context)
    }

    #[test]
    fn light_style_matches_source_ramp() {
        assert_eq!(addon_light_style(0.0, false), "a");
        assert_eq!(addon_light_style(1.0, false), "m");
        assert_eq!(addon_light_style(1.0, true), "z");
        assert_eq!(addon_light_style(0.5, false), "g");
        assert_eq!(addon_light_style(-1.0, true), "a");
    }

    #[test]
    fn lightramp_initializes_and_ramps() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg1);
        let light = game.create("light", None, None).expect("light");
        game.update_entity(&light, |entity| {
            entity.targetname = String::from("ramp");
            entity.fields.insert(String::from("style"), String::from("33"));
        })
        .expect("style");
        let ramp = game.create("target_lightramp", None, None).expect("ramp");
        game.update_entity(&ramp, |entity| {
            entity.target = String::from("ramp");
            entity.targetname = String::from("ramp_use");
        })
        .expect("target");
        spawn_target_lightramp(&mut game, &ramp).expect("spawn");
        assert_eq!(require_entity(&game, &ramp).expect("entity").delay, 1.0);
        ramp_init(&mut game, &ramp).expect("init");
        assert_eq!(require_entity(&game, &ramp).expect("entity").number("ramp.state"), 2.0);
        assert_eq!(require_entity(&game, &ramp).expect("entity").number("cnt"), 1.0);
        ramp_use(&mut game, &ramp, None, None).expect("use");
        assert_eq!(require_entity(&game, &ramp).expect("entity").number("ramp.state"), 3.0);
    }

    #[test]
    fn switchable_light_uses_light_use() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg1);
        let anonymous = game.create("light", None, None).expect("anonymous");
        spawn_light(&mut game, &anonymous).expect("spawn");
        assert!(game.entity_ref(&anonymous).is_none());
        let light = game.create("light", None, None).expect("light");
        game.update_entity(&light, |entity| {
            entity.targetname = String::from("switch");
            entity.fields.insert(String::from("style"), String::from("34"));
        })
        .expect("style");
        spawn_light(&mut game, &light).expect("spawn");
        assert_eq!(
            require_entity(&game, &light).expect("entity").use_callback.as_deref(),
            Some("light_use")
        );
    }

    #[test]
    fn flames_and_candles_become_static() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg1);
        let flame = game.create("light_flame_large_yellow", None, None).expect("flame");
        spawn_flame(&mut game, &flame).expect("spawn");
        assert!(game.entity_ref(&flame).is_none());
        let candle = game.create("light_candle", None, None).expect("candle");
        spawn_light_candle(&mut game, &candle).expect("spawn");
        assert!(game.entity_ref(&candle).is_none());
        let gas = game.create("light_flame_gas", None, None).expect("gas");
        spawn_light_flame_gas(&mut game, &gas).expect("spawn");
        assert!(game.entity_ref(&gas).is_some());
        assert_eq!(
            game.entities
                .values()
                .filter(|entity| entity.classname == "gas_flame")
                .count(),
            1
        );
    }
}
