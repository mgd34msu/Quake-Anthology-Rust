//! Q1 mg3 hanging sacrifice (`src/content/q1/addons/monsters/bosses/sacrifice.ts`).
//!
//! `quakec_mg3/monsters/mg3_sacrifice.qc`. GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

use crate::q1::addons::context::set_addon_vector;
use crate::q1::foundation::callbacks::{Q1ActionHandler, Q1CallbackHandlers, Q1DieHandler, Q1UseHandler};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::monsters::throw_gib;
use crate::q1::foundation::types::{vadd, Q1MoveType, Q1Solid, Q1SoundChannel};
use crate::q1::Q1Error;

/// Gibs a sacrifice, firing its targets (`gib`).
fn sacrifice_gib(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let lowered = vadd(
        game.body(id)?.origin,
        Vec3 {
            x: 0.0,
            y: 0.0,
            z: -32.0,
        },
    );
    game.set_origin(id, lowered)?;
    let origin = game.body(id)?.origin;
    for model in ["gib1", "gib2", "gib3"] {
        throw_gib(game, origin, model, -10.0)?;
    }
    let path = if game.host.random() < 0.5 {
        "player/gib.wav"
    } else {
        "player/udeath.wav"
    };
    game.sound(id, path, Q1SoundChannel::Voice, 0.0, 1.0)?;
    let activator = game.entity_ref(id).and_then(|entity| entity.activator.clone());
    game.use_targets(id, activator.as_ref())?;
    game.remove(id)
}

fn sacrifice_gib_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let activator = activator.cloned();
    game.update_entity(id, |entity| entity.activator = activator)?;
    sacrifice_gib(game, id)
}

fn sacrifice_gib_die(game: &mut Q1EntityServices, id: &ActorId, _attacker: Option<&ActorId>) -> Result<(), Q1Error> {
    sacrifice_gib(game, id)
}

fn sacrifice_animate(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| {
        entity.count += 1.0;
        if entity.count > 75.0 {
            entity.count = 5.0;
        }
        entity.frame = entity.count as i32;
    })?;
    game.schedule(id, 0.1, "mg3:sacrifice_animate")
}

fn sacrifice_think(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let (dest, count) = {
        let entity = game
            .entity_ref(id)
            .ok_or_else(|| crate::q1::q1_error("Missing Q1 sacrifice"))?;
        (entity.vector("dest"), ((entity.count + 0.1) as f32) as f64)
    };
    game.update_entity(id, |entity| entity.count = count)?;
    game.set_origin(
        id,
        vadd(
            dest,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: ((count * 90.0).cos() * 16.0) as f32,
            },
        ),
    )?;
    let angles = vadd(game.body(id)?.angles, Vec3 { x: 0.0, y: 3.6, z: 0.0 });
    game.set_body(
        id,
        &crate::q1::foundation::gameplay::BodyPatch {
            angles: Some(angles),
            ..Default::default()
        },
    )?;
    game.schedule(id, 0.1, "mg3:sacrifice_think")
}

fn spawn_misc_sacrifice(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let use_callback = game.named.use_callback("mg3:sacrifice_gib")?;
    let die = game.named.die("mg3:sacrifice_gib")?;
    let owned = game
        .entity_ref(id)
        .map(|entity| entity.actor.clone())
        .ok_or_else(|| crate::q1::q1_error("Missing Q1 sacrifice"))?;
    game.host.combat.set_health(&owned, 100.0)?;
    let hanging = game.entity_ref(id).is_some_and(|entity| entity.spawnflags & 2 != 0);
    let damageable = game.entity_ref(id).is_some_and(|entity| entity.spawnflags & 1 == 0);
    game.set_damageable(id, damageable)?;
    game.update_entity(id, |entity| {
        entity.use_callback = Some(use_callback);
        entity.die = Some(die);
        entity.max_health = 100.0;
        entity.solid = Q1Solid::Slidebox;
        entity.aimed_damage = damageable;
        entity.movement = Q1MoveType::Step;
    })?;
    if hanging {
        let origin = game.body(id)?.origin;
        game.update_entity(id, |entity| {
            entity.model = String::from("progs/player_hanging.mdl");
            entity.count = 0.0;
            entity.angular_velocity = Vec3 {
                x: 0.0,
                y: 36.0,
                z: 0.0,
            };
        })?;
        set_addon_vector(game, id, "dest", origin)?;
        game.schedule(id, 0.1, "mg3:sacrifice_think")?;
    } else {
        let count = 5.0 + (game.host.random() * 65.0 + 0.5).floor();
        game.update_entity(id, |entity| {
            entity.count = count;
            entity.model = String::from("progs/player_hanging_animated.mdl");
            entity.frame = count as i32;
        })?;
        game.schedule(id, 0.1, "mg3:sacrifice_animate")?;
    }
    game.set_bounds(
        id,
        Bounds {
            min: Vec3 {
                x: -16.0,
                y: -16.0,
                z: -56.0,
            },
            max: Vec3 {
                x: 16.0,
                y: 16.0,
                z: 0.0,
            },
        },
    )
}

/// Registers hanging sacrifices (`registerSacrifice`).
pub fn register_sacrifice(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        "mg3:sacrifice_gib",
        Q1CallbackHandlers {
            use_callback: Some(sacrifice_gib_use as Q1UseHandler),
            die: Some(sacrifice_gib_die as Q1DieHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        "mg3:sacrifice_animate",
        Q1CallbackHandlers {
            action: Some(sacrifice_animate as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        "mg3:sacrifice_think",
        Q1CallbackHandlers {
            action: Some(sacrifice_think as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.register_spawn("misc_sacrifice", spawn_misc_sacrifice)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::{attach_test_player, register_test_addons, Q1AddonProgram};
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::missionpacks::types::test_game;

    fn setup(game: &mut Q1EntityServices) -> Q1BaseGuard {
        let guard = Q1BaseGuard::register(game, Q1BaseOptions::default()).expect("base");
        register_test_addons(game, Q1AddonProgram::Mg3);
        register_sacrifice(game).expect("sacrifice");
        guard
    }

    #[test]
    fn spawn_hangs_or_animates_by_flag() {
        let mut game = test_game();
        let _guard = setup(&mut game);
        let hanging = game.create("misc_sacrifice", None, None).expect("hanging");
        game.update_entity(&hanging, |entity| entity.spawnflags = 2)
            .expect("flags");
        game.spawn_entity(&hanging, None).expect("spawn");
        let entity = game.entity_ref(&hanging).expect("entity");
        assert_eq!(entity.model, "progs/player_hanging.mdl");
        game.invoke_action(&hanging, "mg3:sacrifice_think").expect("think");
        let entity = game.entity_ref(&hanging).expect("entity");
        assert_eq!(entity.count, 0.1_f32 as f64);
        let animated = game.create("misc_sacrifice", None, None).expect("animated");
        game.spawn_entity(&animated, None).expect("spawn");
        let entity = game.entity_ref(&animated).expect("entity");
        assert_eq!(entity.model, "progs/player_hanging_animated.mdl");
        assert!(entity.count >= 5.0 && entity.count <= 70.0);
        game.invoke_action(&animated, "mg3:sacrifice_animate").expect("animate");
    }

    #[test]
    fn gib_use_removes_and_fires_targets() {
        let mut game = test_game();
        let _guard = setup(&mut game);
        let _player = attach_test_player(&mut game);
        let victim = game.create("misc_sacrifice", None, None).expect("victim");
        game.spawn_entity(&victim, None).expect("spawn");
        game.invoke_use(&victim, "mg3:sacrifice_gib", None, None).expect("use");
        assert!(game.entity_ref(&victim).is_none());
    }
}
