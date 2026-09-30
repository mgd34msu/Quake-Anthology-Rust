//! Rogue torches, rubble, and explosion triggers
//! (`src/content/q1/missionpacks/world/rogue-misc.ts`).
//!
//! newmisc.qc entity behavior.

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::q1::base::map_entities::make_static;
use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::{
    BodyPatch, DamageDelivery, DamagePreparation, Q1DamageSourceEffects, TouchSurface,
};
use crate::q1::foundation::types::{length, normalize, vscale, vsub, Q1Event, Q1MoveType, Q1Solid};
use crate::q1::{q1_error, Q1Error};

use super::common::{later, trigger};

/// Gameful `beforeQuad` stage for explosion triggers. The foundation stage
/// closures carry no game access, so the pack registers inert effects and
/// the session damage pipeline calls this method with game access.
#[must_use]
pub fn explosion_trigger_before_quad(
    game: &Q1EntityServices,
    target: &ActorId,
    delivery: DamageDelivery,
    amount: f64,
) -> DamagePreparation {
    let classname = game
        .entity(target)
        .map(|entity| entity.classname.clone())
        .unwrap_or_default();
    if classname == "trigger_explosion" && delivery != DamageDelivery::Radius {
        DamagePreparation::Cancel
    } else {
        DamagePreparation::Continue { amount }
    }
}

/// Damage whoever touches fast rubble.
fn rubble_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let flags = game.entity(other).map(|entity| entity.movement_flags).unwrap_or(0);
    let velocity = game.body(id)?.velocity;
    if (game.is_player(other) || flags & 32 != 0) && f64::from(length(velocity)) > 0.0 {
        let id_copy = id.clone();
        let other = other.clone();
        game.damage(
            &other,
            Some(&id_copy),
            Some(&id_copy),
            10.0,
            &crate::q1::foundation::entity_services::Q1DamageParams::default(),
        );
    }
    Ok(())
}

/// Throw one rubble chunk at the target.
fn rubble_throw(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let target = game.entity(id).map(|entity| entity.target.clone()).unwrap_or_default();
    let destination = game
        .find(&target)
        .first()
        .cloned()
        .or_else(|| game.world.clone())
        .filter(|destination| game.entity(destination).is_some());
    let Some(destination) = destination else {
        return Err(q1_error("Rubble generator requires worldspawn"));
    };
    let origin = game.body(id)?.origin;
    let direction = normalize(vsub(game.body(&destination)?.origin, origin));
    let rubble = game.create("rubble", None, None)?;
    let spawnflags = game.entity(id).map(|entity| entity.spawnflags).unwrap_or(0);
    let owner = id.clone();
    game.update_entity(&rubble, |rubble| {
        rubble.owner = Some(owner);
        rubble.model = "progs/rubble.mdl".to_string();
        rubble.solid = Q1Solid::Bbox;
        rubble.movement = Q1MoveType::Bounce;
        rubble.skin = if spawnflags & 1 != 0 { 1 } else { 0 };
    })?;
    let velocity = vscale(
        Vec3 {
            x: (f64::from(direction.x) + game.host.random() * 0.2 - 0.1) as f32,
            y: (f64::from(direction.y) + game.host.random() * 0.2 - 0.1) as f32,
            z: (f64::from(direction.z) + game.host.random() * 0.2 - 0.1) as f32,
        },
        300.0,
    );
    game.set_body(
        &rubble,
        &BodyPatch {
            origin: Some(origin),
            velocity: Some(velocity),
            bounds: Some(qa_core::math::Bounds {
                min: Vec3 {
                    x: -16.0,
                    y: -16.0,
                    z: -16.0,
                },
                max: Vec3 {
                    x: 16.0,
                    y: 16.0,
                    z: 16.0,
                },
            }),
            ..Default::default()
        },
    )?;
    let touch_name = game.named.touch("rogue:rubble_touch")?;
    game.update_entity(&rubble, |rubble| rubble.touch = Some(touch_name))?;
    later(game, &rubble, 30.0, "SUB_Remove")?;
    game.link(&rubble)?;
    let delay = game.entity(id).map(|entity| entity.delay).unwrap_or(0.0);
    later(game, id, delay, "rogue:rubble_throw")
}

/// Toggle a rubble generator.
fn rubble_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    if game.entity(id).map(|entity| entity.wait).unwrap_or(0.0) == 0.0 {
        game.update_entity(id, |entity| entity.wait = 1.0)?;
        let delay = game.entity(id).map(|entity| entity.delay).unwrap_or(0.0);
        return later(game, id, delay, "rogue:rubble_throw");
    }
    game.update_entity(id, |entity| entity.wait = 0.0)?;
    game.cancel(id);
    Ok(())
}

/// Fire explosion-trigger targets on death.
fn explosion_trigger_die(game: &mut Q1EntityServices, id: &ActorId, attacker: Option<&ActorId>) -> Result<(), Q1Error> {
    let attacker = attacker.cloned();
    game.use_targets(id, attacker.as_ref())?;
    game.update_entity(id, |entity| entity.touch = None)?;
    later(game, id, 0.1, "SUB_Remove")
}

/// Replace `light_torch_small_walltorch` with a flame and fire loop.
fn spawn_walltorch(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let model = game.precache_model("progs/flame.mdl")?;
    game.update_entity(id, |entity| entity.model = model)?;
    if game.entity(id).map(|entity| entity.spawnflags).unwrap_or(0) & 1 == 0 {
        let path = game.precache_sound("ambience/fire1.wav")?;
        let origin = game.body(id)?.origin;
        game.host.emit(Q1Event::Ambient {
            origin,
            path,
            volume: 0.5,
            attenuation: 3.0,
        });
    }
    make_static(game, id)
}

/// Spawn a `light_lantern` or `light_candle` static.
fn spawn_light_model(model: &'static str) -> fn(&mut Q1EntityServices, &ActorId) -> Result<(), Q1Error> {
    if model == "lantern" {
        spawn_lantern
    } else {
        spawn_candle
    }
}

fn light_model(game: &mut Q1EntityServices, id: &ActorId, model: &str) -> Result<(), Q1Error> {
    let model = game.precache_model(&format!("progs/{model}.mdl"))?;
    game.update_entity(id, |entity| {
        entity.model = model;
        entity.solid = Q1Solid::None;
        entity.movement = Q1MoveType::None;
    })?;
    make_static(game, id)
}

fn spawn_lantern(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    light_model(game, id, "lantern")
}

fn spawn_candle(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    light_model(game, id, "candle")
}

/// Spawn a `rubble_generator`.
fn spawn_rubble_generator(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if game
        .entity(id)
        .map(|entity| entity.target.clone())
        .unwrap_or_default()
        .is_empty()
    {
        return Err(q1_error("rubble_generator has no target!"));
    }
    game.update_entity(id, |entity| {
        if entity.delay == 0.0 {
            entity.delay = 5.0;
        }
        entity.solid = Q1Solid::None;
    })?;
    let use_name = game.named.use_callback("rogue:rubble_use")?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_name))?;
    if game.entity(id).map(|entity| entity.spawnflags).unwrap_or(0) & 2 != 0 {
        let name = game
            .entity(id)
            .and_then(|entity| entity.use_callback.clone())
            .unwrap_or_default();
        return game.invoke_use(id, &name, None, None);
    }
    Ok(())
}

/// Spawn a `trigger_explosion`.
fn spawn_trigger_explosion(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    trigger(game, id)?;
    let health = game.health(id);
    game.update_entity(id, |entity| {
        entity.max_health = if health == 0.0 { 20.0 } else { health };
    })?;
    let max_health = game.entity(id).map(|entity| entity.max_health).unwrap_or(0.0);
    game.set_health(id, max_health)?;
    let die_name = game.named.die("rogue:explosion_trigger_die")?;
    game.update_entity(id, |entity| entity.die = Some(die_name))?;
    game.set_damageable(id, true)?;
    game.update_entity(id, |entity| entity.solid = Q1Solid::Bbox)?;
    game.link(id)
}

/// Inert explosion-trigger damage effects. The donor `beforeQuad` logic
/// lives in the gameful [`explosion_trigger_before_quad`], which the
/// session damage pipeline calls with game access.
fn explosion_trigger_effects() -> Q1DamageSourceEffects {
    Q1DamageSourceEffects {
        before_quad: None,
        after_quad: None,
        armor_allowed: None,
        protection_applies: None,
        before_health: None,
        after_armor: None,
        lethal_health: None,
    }
}

/// Register Rogue misc entities (`registerRogueMisc`).
pub fn register_rogue_misc(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.replace_spawn("light_torch_small_walltorch", spawn_walltorch)?;
    game.register_spawn("light_lantern", spawn_light_model("lantern"))?;
    game.register_spawn("light_candle", spawn_light_model("candle"))?;
    game.named.register(
        "rogue:rubble_touch",
        Q1CallbackHandlers {
            touch: Some(rubble_touch),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:rubble_throw",
        Q1CallbackHandlers {
            action: Some(rubble_throw),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:rubble_use",
        Q1CallbackHandlers {
            use_callback: Some(rubble_use),
            ..Default::default()
        },
    )?;
    game.register_spawn("rubble_generator", spawn_rubble_generator)?;
    game.named.register(
        "rogue:explosion_trigger_die",
        Q1CallbackHandlers {
            die: Some(explosion_trigger_die),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_explosion", spawn_trigger_explosion)?;
    game.register_damage_source_effects("rogue:explosion-trigger", explosion_trigger_effects())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::foundation::host::mock::MockEvents;
    use crate::q1::missionpacks::types::test_game_with_events;

    fn game_with_base() -> (
        Box<Q1EntityServices>,
        Q1BaseGuard,
        std::rc::Rc<std::cell::RefCell<MockEvents>>,
    ) {
        let (game, events) = test_game_with_events();
        let mut game = Box::new(game);
        let guard = Q1BaseGuard::register(&mut game, Q1BaseOptions::default()).expect("base");
        register_rogue_misc(&mut game).expect("register");
        (game, guard, events)
    }

    #[test]
    fn walltorch_replacement_sets_flame_model() {
        let (mut game, _guard, events) = game_with_base();
        let id = game.create("light_torch_small_walltorch", None, None).expect("torch");
        game.spawn_entity(&id, None).expect("spawn");
        assert!(game.entity(&id).is_none());
        assert!(events.borrow().events.iter().any(|event| matches!(
            event,
            Q1Event::StaticModel { path, .. } if path == "progs/flame.mdl"
        )));
    }

    #[test]
    fn rubble_generator_throws_on_use() {
        let (mut game, _guard, _events) = game_with_base();
        let target = game.create("info_null", None, None).expect("target");
        game.update_entity(&target, |entity| entity.targetname = "t1".to_string())
            .expect("targetname");
        let id = game.create("rubble_generator", None, None).expect("generator");
        game.update_entity(&id, |entity| entity.target = "t1".to_string())
            .expect("target");
        game.spawn_entity(&id, None).expect("spawn");
        game.invoke_use(&id, "rogue:rubble_use", None, None).expect("use");
        assert_eq!(game.entity(&id).expect("generator").wait, 1.0);
        game.invoke_action(&id, "rogue:rubble_throw").expect("throw");
        let chunks = game
            .entity_ids()
            .into_iter()
            .filter(|id| game.entity(id).is_some_and(|entity| entity.classname == "rubble"));
        assert_eq!(chunks.count(), 1);
    }

    #[test]
    fn explosion_trigger_quad_stage_cancels_direct_only() {
        let (mut game, _guard, _events) = game_with_base();
        let id = game.create("trigger_explosion", None, None).expect("trigger");
        game.spawn_entity(&id, None).expect("spawn");
        assert_eq!(
            explosion_trigger_before_quad(&game, &id, DamageDelivery::Direct, 50.0),
            DamagePreparation::Cancel
        );
        assert_eq!(
            explosion_trigger_before_quad(&game, &id, DamageDelivery::Radius, 50.0),
            DamagePreparation::Continue { amount: 50.0 }
        );
    }
}
