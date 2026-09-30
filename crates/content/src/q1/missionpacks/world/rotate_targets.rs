//! Hipnotic rotation target transforms
//! (`src/content/q1/missionpacks/world/rotate-targets.ts`).
//!
//! hiprot.qc target transforms shared by the rotation entities.

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::BodyPatch;
use crate::q1::foundation::types::{vadd, vectors, vscale, vsub, ZERO};
use crate::q1::missionpacks::types::fround;
use crate::q1::{q1_error, Q1Error};

use super::common::{number, vector};

/// Wrap angles into `[0, 360)` per component (`normalizeAngles`).
#[must_use]
pub fn normalize_angles(angles: Vec3) -> Vec3 {
    let normalize = |angle: f32| -> f32 {
        let angle = f64::from(angle);
        fround(angle - (angle / 360.0).floor() * 360.0) as f32
    };
    Vec3 {
        x: normalize(angles.x),
        y: normalize(angles.y),
        z: normalize(angles.z),
    }
}

/// Bind rotation targets to a rotator (`linkRotateTargets`).
pub fn link_rotate_targets(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let origin = game.body(id)?.origin;
    game.update_entity(id, |entity| vector(entity, "oldorigin", origin))?;
    let target = game.entity(id).map(|entity| entity.target.clone()).unwrap_or_default();
    for target in game.find(&target) {
        let (body, classname) = game
            .entity(&target)
            .map(|entity| (game.body(&target), entity.classname.clone()))
            .ok_or_else(|| q1_error("Missing Q1 entity"))?;
        let body = body?;
        let wall = classname == "func_movewall";
        let center = if wall {
            vadd(body.origin, vscale(vadd(body.bounds.min, body.bounds.max), 0.5))
        } else {
            body.origin
        };
        let relative = vsub(center, origin);
        let rotate_type = if wall {
            1.0
        } else if classname == "rotate_object" {
            0.0
        } else {
            2.0
        };
        let owner = if wall || classname == "rotate_object" {
            Some(id.clone())
        } else {
            None
        };
        game.update_entity(&target, |entity| {
            vector(entity, "oldorigin", relative);
            vector(entity, "neworigin", relative);
            number(entity, "rotate_type", rotate_type);
            if let Some(owner) = owner.clone() {
                entity.owner = Some(owner);
            }
        })?;
    }
    Ok(())
}

/// Move rotation targets along the current angles (`rotateTargets`).
pub fn rotate_targets(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let body = game.body(id)?;
    let basis = vectors(body.angles);
    let target = game.entity(id).map(|entity| entity.target.clone()).unwrap_or_default();
    let old_origin = game.entity(id).map(|entity| entity.vector("oldorigin")).unwrap_or(ZERO);
    for target in game.find(&target) {
        let (old, rotate_type) = game
            .entity(&target)
            .map(|entity| (entity.vector("oldorigin"), entity.number("rotate_type")))
            .unwrap_or((ZERO, 2.0));
        let next = vadd(
            vadd(
                vscale(basis.forward, f64::from(old.x)),
                vscale(basis.right, f64::from(-old.y)),
            ),
            vscale(basis.up, f64::from(old.z)),
        );
        if rotate_type == 1.0 {
            let next = vadd(vsub(body.origin, old_origin), vsub(next, old));
            let target_origin = game.body(&target)?.origin;
            game.update_entity(&target, |entity| vector(entity, "neworigin", next))?;
            game.set_body(
                &target,
                &BodyPatch {
                    velocity: Some(vscale(vsub(next, target_origin), 25.0)),
                    ..Default::default()
                },
            )?;
        } else {
            game.update_entity(&target, |entity| vector(entity, "neworigin", next))?;
            if rotate_type == 0.0 {
                game.set_body(
                    &target,
                    &BodyPatch {
                        angles: Some(body.angles),
                        ..Default::default()
                    },
                )?;
            }
            game.set_origin(&target, vadd(next, body.origin))?;
        }
    }
    Ok(())
}

/// Stop rotation targets, syncing `rotate_object` angles (`rotateTargetsFinal`).
pub fn rotate_targets_final(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let body = game.body(id)?;
    let target = game.entity(id).map(|entity| entity.target.clone()).unwrap_or_default();
    for target in game.find(&target) {
        let rotate_type = game
            .entity(&target)
            .map(|entity| entity.number("rotate_type"))
            .unwrap_or(2.0);
        game.set_body(
            &target,
            &BodyPatch {
                velocity: Some(ZERO),
                angles: if rotate_type == 0.0 { Some(body.angles) } else { None },
                ..Default::default()
            },
        )?;
    }
    Ok(())
}

/// Snap rotation targets to their saved offsets (`setTargetOrigin`).
pub fn set_target_origin(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let body = game.body(id)?;
    let (target, old_origin) = game
        .entity(id)
        .map(|entity| (entity.target.clone(), entity.vector("oldorigin")))
        .unwrap_or_default();
    for target in game.find(&target) {
        let (rotate_type, new_origin, target_old) = game
            .entity(&target)
            .map(|entity| {
                (
                    entity.number("rotate_type"),
                    entity.vector("neworigin"),
                    entity.vector("oldorigin"),
                )
            })
            .unwrap_or((2.0, ZERO, ZERO));
        game.set_origin(
            &target,
            if rotate_type == 1.0 {
                vadd(vsub(body.origin, old_origin), vsub(new_origin, target_old))
            } else {
                vadd(new_origin, body.origin)
            },
        )?;
    }
    Ok(())
}

/// Copy crush damage onto hurt/movewall targets (`damageOnTargets`).
pub fn damage_on_targets(game: &mut Q1EntityServices, id: &ActorId, damage: f64) -> Result<(), Q1Error> {
    let target = game.entity(id).map(|entity| entity.target.clone()).unwrap_or_default();
    for target in game.find(&target) {
        let classname = game
            .entity(&target)
            .map(|entity| entity.classname.clone())
            .unwrap_or_default();
        if classname == "trigger_hurt" || classname == "func_movewall" {
            game.update_entity(&target, |entity| entity.damage = damage)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::missionpacks::types::test_game;

    fn linked_pair() -> (Q1EntityServices, ActorId, ActorId) {
        let mut game = test_game();
        let rotator = game.create("func_rotate_entity", None, None).expect("rotator");
        let target = game.create("rotate_object", None, None).expect("target");
        game.update_entity(&rotator, |entity| entity.target = "t1".to_string())
            .expect("target");
        game.update_entity(&target, |entity| entity.targetname = "t1".to_string())
            .expect("targetname");
        game.set_origin(
            &target,
            Vec3 {
                x: 64.0,
                y: 0.0,
                z: 0.0,
            },
        )
        .expect("origin");
        link_rotate_targets(&mut game, &rotator).expect("link");
        (game, rotator, target)
    }

    #[test]
    fn normalize_angles_wraps_full_turns() {
        let wrapped = normalize_angles(Vec3 {
            x: 370.0,
            y: -90.0,
            z: 720.0,
        });
        assert_eq!(wrapped.x, 10.0);
        assert_eq!(wrapped.y, 270.0);
        assert_eq!(wrapped.z, 0.0);
    }

    #[test]
    fn link_and_rotate_moves_object_targets() {
        let (mut game, rotator, target) = linked_pair();
        let linked = game.entity(&target).cloned().expect("target");
        assert_eq!(linked.number("rotate_type"), 0.0);
        assert_eq!(
            linked.vector("oldorigin"),
            Vec3 {
                x: 64.0,
                y: 0.0,
                z: 0.0
            }
        );
        game.set_body(
            &rotator,
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
        rotate_targets(&mut game, &rotator).expect("rotate");
        let moved = game.body(&target).expect("body");
        assert!((f64::from(moved.origin.x) - 0.0).abs() < 0.01);
        assert!((f64::from(moved.origin.y) - 64.0).abs() < 0.01);
        rotate_targets_final(&mut game, &rotator).expect("final");
        assert_eq!(game.body(&target).expect("body").velocity, ZERO);
    }

    #[test]
    fn damage_on_targets_only_touches_hurt_walls() {
        let mut game = test_game();
        let rotator = game.create("func_rotate_entity", None, None).expect("rotator");
        let hurt = game.create("trigger_hurt", None, None).expect("hurt");
        let other = game.create("rotate_object", None, None).expect("other");
        game.update_entity(&rotator, |entity| entity.target = "t1".to_string())
            .expect("target");
        for id in [&hurt, &other] {
            game.update_entity(id, |entity| entity.targetname = "t1".to_string())
                .expect("targetname");
        }
        damage_on_targets(&mut game, &rotator, 25.0).expect("damage");
        assert_eq!(game.entity(&hurt).expect("hurt").damage, 25.0);
        assert_eq!(game.entity(&other).expect("other").damage, 0.0);
    }
}
