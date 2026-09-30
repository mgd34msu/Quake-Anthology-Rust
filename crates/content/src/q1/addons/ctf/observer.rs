//! Q1 CTF observer movement (src/content/q1/addons/ctf/observer.ts).

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

use crate::q1::addons::ctf::state::{ctf_body, ctf_number, ctf_owner, ctf_set, ctf_write_body, with_ctf_services};
use crate::q1::addons::ctf::teams::spawn_point;
use crate::q1::foundation::entity::Q1MoverState;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::{BodyPatch, CombatTraits};
use crate::q1::foundation::types::{dot, length, normalize, vadd, vectors, vscale, vsub, ZERO};
use crate::q1::Q1Error;

/// Observer collision bounds.
const OBSERVER_BOUNDS: Bounds = Bounds {
    min: Vec3 {
        x: -12.0,
        y: -12.0,
        z: -12.0,
    },
    max: Vec3 {
        x: 12.0,
        y: 12.0,
        z: 12.0,
    },
};

/// Admit an actor as an observer (`becomeObserver`). The selected
/// movement host owns observer admission and collision.
pub fn become_observer(game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
    let id = actor.clone();
    with_ctf_services(game, |services| services.set_observer(&id, true))?;
    let owner = ctf_owner(game, &id)?;
    game.host.combat.set_health(&owner, 999.0)?;
    let Some(combat) = game.host.combat.read(&id) else {
        return ctf_write_body(
            game,
            &id,
            &BodyPatch {
                bounds: Some(OBSERVER_BOUNDS),
                ..Default::default()
            },
        );
    };
    game.host.combat.set_traits(
        &owner,
        CombatTraits {
            can_take_damage: false,
            mass: combat.mass,
            invulnerable: combat.invulnerable,
            team: combat.team,
            no_knockback: combat.no_knockback,
        },
    )?;
    ctf_write_body(
        game,
        &id,
        &BodyPatch {
            bounds: Some(OBSERVER_BOUNDS),
            ..Default::default()
        },
    )
}

/// Pass an observer through a door volume (`throughDoor`).
fn through_door(game: &mut Q1EntityServices, actor: &ActorId, door: &ActorId) -> Result<(), Q1Error> {
    let door_entity = match game.entity_ref(door).cloned() {
        Some(entity) => entity,
        None => return Ok(()),
    };
    let master = match door_entity.door_group.first() {
        Some(master) => Some(master.clone()),
        None => door_entity.owner.clone(),
    };
    let Some(master) = master else {
        return Ok(());
    };
    let master_entity = match game.entity_ref(&master).cloned() {
        Some(entity) => entity,
        None => return Ok(()),
    };
    if master_entity.state != Q1MoverState::Bottom {
        return Ok(());
    }
    let members: Vec<ActorId> = if master_entity.door_group.is_empty() {
        vec![master]
    } else {
        master_entity.door_group.clone()
    };
    let mut min = Vec3 {
        x: f32::INFINITY,
        y: f32::INFINITY,
        z: f32::INFINITY,
    };
    let mut max = Vec3 {
        x: f32::NEG_INFINITY,
        y: f32::NEG_INFINITY,
        z: f32::NEG_INFINITY,
    };
    for member in &members {
        let body = game.body(member)?;
        let low = vadd(body.origin, body.bounds.min);
        let high = vadd(body.origin, body.bounds.max);
        min = Vec3 {
            x: min.x.min(low.x),
            y: min.y.min(low.y),
            z: min.z.min(low.z),
        };
        max = Vec3 {
            x: max.x.max(high.x),
            y: max.y.max(high.y),
            z: max.z.max(high.z),
        };
    }
    let body = ctf_body(game, actor)?;
    let low = vadd(body.origin, body.bounds.min);
    let high = vadd(body.origin, body.bounds.max);
    let x = min.x + 15.0 < low.x && high.x < max.x - 15.0;
    let y = min.y + 15.0 < low.y && high.y < max.y - 15.0;
    let z = min.z + 15.0 < low.z && high.z < max.z - 15.0;
    let mut direction = ZERO;
    let mut origin = body.origin;
    if x && y {
        if origin.z < min.z {
            direction = Vec3 { x: 0.0, y: 0.0, z: 1.0 };
            origin = Vec3 {
                z: max.z + 25.0,
                ..origin
            };
        } else if origin.z > max.z {
            direction = Vec3 {
                x: 0.0,
                y: 0.0,
                z: -1.0,
            };
            origin = Vec3 {
                z: min.z - 25.0,
                ..origin
            };
        }
    } else if x && z {
        if origin.y < min.y {
            direction = Vec3 { x: 0.0, y: 1.0, z: 0.0 };
            origin = Vec3 {
                y: max.y + 25.0,
                ..origin
            };
        } else if origin.y > max.y {
            direction = Vec3 {
                x: 0.0,
                y: -1.0,
                z: 0.0,
            };
            origin = Vec3 {
                y: min.y - 25.0,
                ..origin
            };
        }
    } else if y && z {
        if origin.x < min.x {
            direction = Vec3 { x: 1.0, y: 0.0, z: 0.0 };
            origin = Vec3 {
                x: max.x + 25.0,
                ..origin
            };
        } else if origin.x > max.x {
            direction = Vec3 {
                x: -1.0,
                y: 0.0,
                z: 0.0,
            };
            origin = Vec3 {
                x: min.x - 25.0,
                ..origin
            };
        }
    }
    if dot(direction, normalize(body.velocity)) >= 0.5 {
        ctf_write_body(
            game,
            actor,
            &BodyPatch {
                origin: Some(origin),
                ..Default::default()
            },
        )?;
    }
    Ok(())
}

/// Pass an observer through a teleporter (`throughTeleporter`).
fn through_teleporter(game: &mut Q1EntityServices, actor: &ActorId, teleporter: &ActorId) -> Result<(), Q1Error> {
    let id = actor.clone();
    let body = ctf_body(game, &id)?;
    let tele_body = game.body(teleporter)?;
    let direction = vsub(
        vadd(
            tele_body.origin,
            vscale(vadd(tele_body.bounds.min, tele_body.bounds.max), 0.5),
        ),
        body.origin,
    );
    // The QC calls normalize without assigning its return value here.
    if dot(direction, body.velocity) <= 0.1 {
        return Ok(());
    }
    let target = game
        .entity_ref(teleporter)
        .map(|entity| entity.target.clone())
        .unwrap_or_default();
    let Some(target) = game.find(&target).into_iter().next() else {
        return Ok(());
    };
    let target_body = game.body(&target)?;
    let mangle = game.entity_ref(&target).map(|entity| entity.mangle).unwrap_or(ZERO);
    let forward = vectors(mangle).forward;
    let until = game.time + 0.7;
    with_ctf_services(game, |services| {
        services.teleport(&id, target_body.origin, mangle, vscale(forward, 300.0), until);
    })?;
    Ok(())
}

/// Run observer movement for a frame (`observerFrame`).
pub fn observer_frame(game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
    let id = actor.clone();
    let body = ctf_body(game, &id)?;
    let input = with_ctf_services(game, |services| services.input(&id))?;
    let forward = vectors(input.view_angles).forward;
    let horizontal = Vec3 {
        x: forward.x,
        y: forward.y,
        z: 0.0,
    };
    let cosine = length(horizontal);
    let inverse = if cosine == 0.0 { 0.0 } else { 1.0 / cosine };
    let facing = vscale(horizontal, f64::from(inverse));
    let velocity = Vec3 {
        z: 0.0,
        ..body.velocity
    };
    let parallel = vscale(facing, f64::from(dot(facing, velocity)));
    let strafe = vsub(velocity, parallel);
    let along = if dot(facing, velocity) < 0.0 {
        -length(parallel)
    } else {
        length(parallel)
    };
    let mut projected = vscale(forward, f64::from(along * inverse));
    projected = Vec3 {
        z: projected.z + body.velocity.z * 0.75,
        ..projected
    };
    let speed = length(projected);
    let maximum = 320.0 - 100.0 * forward.z;
    if speed > maximum {
        projected = vscale(projected, f64::from(maximum / speed));
    }
    if body.angles.x.abs() == 30.0 {
        projected = Vec3 {
            z: -projected.z,
            ..projected
        };
    }
    ctf_write_body(
        game,
        &id,
        &BodyPatch {
            velocity: Some(vadd(projected, strafe)),
            ..Default::default()
        },
    )?;
    for entity in game.entity_ids() {
        let target = game.body(&entity)?;
        let center = vadd(target.origin, vscale(vadd(target.bounds.min, target.bounds.max), 0.5));
        if length(vsub(center, body.origin)) > 75.0 {
            continue;
        }
        let classname = game
            .entity_ref(&entity)
            .map(|entity| entity.classname.clone())
            .unwrap_or_default();
        if classname == "func_door" {
            through_door(game, &id, &entity)?;
            break;
        }
        if classname == "trigger_teleport" {
            through_teleporter(game, &id, &entity)?;
            break;
        }
    }
    if input.jump && ctf_number(game, &id, "observerJumpHeld")? == 0.0 {
        if let Some(spot) = spawn_point(game, &id)? {
            let spot_body = game.body(&spot)?;
            let velocity = ctf_body(game, &id)?.velocity;
            let origin = vadd(spot_body.origin, Vec3 { x: 0.0, y: 0.0, z: 1.0 });
            with_ctf_services(game, |services| {
                services.teleport(&id, origin, spot_body.angles, velocity, input.teleport_until);
            })?;
        }
    }
    ctf_set(game, &id, "observerJumpHeld", f64::from(u8::from(input.jump)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::{attach_test_player, register_test_addons, Q1AddonProgram};
    use crate::q1::addons::ctf::state::register_ctf_state;
    use crate::q1::addons::ctf::types::FakeCtfServices;
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::missionpacks::types::test_game;

    fn setup(game: &mut Q1EntityServices) -> (Q1BaseGuard, ActorId) {
        let guard = Q1BaseGuard::register(game, Q1BaseOptions::default()).expect("base");
        register_test_addons(game, Q1AddonProgram::Ctf);
        let (services, _) = FakeCtfServices::new();
        register_ctf_state(game, Box::new(services), true, false);
        let player = attach_test_player(game);
        (guard, player)
    }

    #[test]
    fn become_observer_protects_and_shrinks() {
        let mut game = test_game();
        let (_guard, player) = setup(&mut game);
        become_observer(&mut game, &player).expect("observer");
        assert_eq!(game.health(&player), 999.0);
        let combat = game.host.combat.read(&player).expect("combat");
        assert!(!combat.can_take_damage);
        let body = ctf_body(&game, &player).expect("body");
        assert_eq!(body.bounds.min.x, -12.0);
        assert!(with_ctf_services(&game, |services| services.observer(&player)).expect("flag"));
    }

    #[test]
    fn observer_frame_clamps_speed() {
        let mut game = test_game();
        let (_guard, player) = setup(&mut game);
        become_observer(&mut game, &player).expect("observer");
        ctf_write_body(
            &mut game,
            &player,
            &BodyPatch {
                velocity: Some(Vec3 {
                    x: 1000.0,
                    y: 0.0,
                    z: 0.0,
                }),
                ..Default::default()
            },
        )
        .expect("velocity");
        observer_frame(&mut game, &player).expect("frame");
        let body = ctf_body(&game, &player).expect("body");
        assert!(length(body.velocity) <= 320.0 + f32::EPSILON);
    }
}
