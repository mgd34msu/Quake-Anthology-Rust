//! Mission-pack world entity helpers
//! (`src/content/q1/missionpacks/world/common.ts`).
//!
//! Mission pack QuakeC field/scheduling helpers shared by every world leaf.

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::q1::foundation::entity::{move_direction, Q1Actor};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::BodyPatch;
use crate::q1::foundation::types::{Q1MoveType, Q1Solid, ZERO};
use crate::q1::missionpacks::types::fround;
use crate::q1::{q1_error, Q1Error};

/// Store a binary32 numeric field (`number`).
pub fn number(entity: &mut Q1Actor, key: &str, value: f64) {
    entity.fields.insert(key.to_string(), fround(value).to_string());
}

/// Store a binary32 vector field (`vector`).
pub fn vector(entity: &mut Q1Actor, key: &str, value: Vec3) {
    entity.fields.insert(
        key.to_string(),
        format!(
            "{} {} {}",
            fround(f64::from(value.x)),
            fround(f64::from(value.y)),
            fround(f64::from(value.z))
        ),
    );
}

/// Schedule a named action after a delay (`later`).
pub fn later(game: &mut Q1EntityServices, id: &ActorId, delay: f64, name: &str) -> Result<(), Q1Error> {
    game.schedule(id, delay, name)
}

/// Configure a trigger volume from shared spawn angles (`trigger`).
pub fn trigger(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let angles = game.body(id)?.angles;
    let movedir = move_direction(angles, Some(game));
    game.update_entity(id, |entity| {
        entity.movedir = movedir;
        entity.solid = Q1Solid::Trigger;
        entity.model.clear();
    })?;
    game.set_body(
        id,
        &BodyPatch {
            angles: Some(ZERO),
            ..Default::default()
        },
    )
}

/// Configure a pusher brush, saving spawn angles as mangle (`brush`).
pub fn brush(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let angles = game.body(id)?.angles;
    game.update_entity(id, |entity| {
        entity.mangle = angles;
        entity.solid = Q1Solid::Bsp;
        entity.movement = Q1MoveType::Push;
    })?;
    game.set_body(
        id,
        &BodyPatch {
            angles: Some(ZERO),
            ..Default::default()
        },
    )
}

/// Fire `useTargets` at an override target, restoring the entity's own
/// target/message afterwards (`targetEvent`).
pub fn target_event(game: &mut Q1EntityServices, id: &ActorId, target: &str, message: &str) -> Result<(), Q1Error> {
    let (prior_target, prior_message, activator) = game
        .entity(id)
        .map(|entity| (entity.target.clone(), entity.message.clone(), entity.activator.clone()))
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let target = target.to_string();
    let message = message.to_string();
    game.update_entity(id, |entity| {
        entity.target.clone_from(&target);
        entity.message.clone_from(&message);
    })?;
    let result = game.use_targets(id, activator.as_ref());
    if game.entity(id).is_some() {
        game.update_entity(id, |entity| {
            entity.target = prior_target;
            entity.message = prior_message;
        })?;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::foundation::callbacks::Q1CallbackHandlers;
    use crate::q1::missionpacks::types::test_game;

    #[test]
    fn number_and_vector_round_like_fround() {
        let mut game = test_game();
        let id = game.create("test", None, None).expect("create");
        game.update_entity(&id, |entity| {
            number(entity, "count", 0.1);
            vector(
                entity,
                "origin",
                Vec3 {
                    x: 0.1,
                    y: 2.0,
                    z: -3.5,
                },
            );
        })
        .expect("update");
        let entity = game.entity(&id).cloned().expect("entity");
        assert_eq!(entity.fields.get("count").expect("count"), &fround(0.1).to_string());
        assert_eq!(
            entity.vector("origin"),
            Vec3 {
                x: 0.1,
                y: 2.0,
                z: -3.5
            }
        );
    }

    #[test]
    fn later_schedules_named_action() {
        let mut game = test_game();
        let id = game.create("test", None, None).expect("create");
        later(&mut game, &id, 0.5, "SUB_Null").expect("later");
        let entity = game.entity(&id).cloned().expect("entity");
        assert_eq!(entity.think.as_deref(), Some("SUB_Null"));
        assert_eq!(entity.next_think, game.time + 0.5);
    }

    #[test]
    fn trigger_and_brush_configure_solids() {
        let mut game = test_game();
        let trigger_id = game.create("trigger", None, None).expect("trigger");
        game.set_body(
            &trigger_id,
            &BodyPatch {
                angles: Some(Vec3 {
                    x: 0.0,
                    y: 90.0,
                    z: 0.0,
                }),
                ..Default::default()
            },
        )
        .expect("angles");
        trigger(&mut game, &trigger_id).expect("trigger");
        let entity = game.entity(&trigger_id).cloned().expect("entity");
        assert_eq!(entity.solid, Q1Solid::Trigger);
        assert!(entity.model.is_empty());
        assert_eq!(game.body(&trigger_id).expect("body").angles, ZERO);

        let brush_id = game.create("brush", None, None).expect("brush");
        brush(&mut game, &brush_id).expect("brush");
        let entity = game.entity(&brush_id).cloned().expect("entity");
        assert_eq!(entity.solid, Q1Solid::Bsp);
        assert_eq!(entity.movement, Q1MoveType::Push);
    }

    #[test]
    fn target_event_restores_target_and_message() {
        fn mark_used(
            game: &mut Q1EntityServices,
            id: &ActorId,
            _: Option<&ActorId>,
            _: Option<&ActorId>,
        ) -> Result<(), Q1Error> {
            game.update_entity(id, |entity| {
                entity.fields.insert("used".to_string(), "1".to_string());
            })
        }

        let mut game = test_game();
        game.named
            .register(
                "test:mark_used",
                Q1CallbackHandlers {
                    use_callback: Some(mark_used),
                    ..Default::default()
                },
            )
            .expect("register");
        let target = game.create("target", None, None).expect("target");
        let use_name = game.named.use_callback("test:mark_used").expect("use name");
        game.update_entity(&target, |entity| {
            entity.targetname = "t1".to_string();
            entity.use_callback = Some(use_name);
        })
        .expect("targetname");
        let source = game.create("source", None, None).expect("source");
        game.update_entity(&source, |entity| {
            entity.target = "kept".to_string();
            entity.message = "msg".to_string();
        })
        .expect("source");
        target_event(&mut game, &source, "t1", "").expect("fire");
        assert_eq!(game.entity(&target).expect("target").text("used"), "1".to_string());
        let source = game.entity(&source).cloned().expect("source");
        assert_eq!(source.target, "kept");
        assert_eq!(source.message, "msg");
    }
}
