//! Hipnotic particle fields, toggle walls, and wall sprites
//! (`src/content/q1/missionpacks/world/hipnotic-particles.ts`).
//!
//! hip_part.qc / hipholes.qc entity behavior.

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::q1::base::map_entities::make_static;
use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::{BodyPatch, TouchSurface};
use crate::q1::foundation::types::{vadd, vectors, vscale, vsub, Q1Event, Q1MoveType, Q1Solid, ZERO};
use crate::q1::{q1_error, Q1Error};

use super::common::number;

/// Emit one particle-field burst (`fieldUse`).
fn field_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let (spawnflags, cnt) = game
        .entity(id)
        .map(|entity| (entity.spawnflags, entity.number("cnt")))
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    if spawnflags & 1 != 0 {
        let counter = other
            .and_then(|other| game.entity(other))
            .map(|counter| {
                if counter.classname == "func_counter" {
                    counter.number("counter_state")
                } else {
                    0.0
                }
            })
            .unwrap_or(0.0);
        if counter != cnt {
            return Ok(());
        }
    }
    let time = game.time;
    game.update_entity(id, |entity| number(entity, "ltime", time + 0.25))?;
    let noise = game.entity(id).map(|entity| entity.text("noise")).unwrap_or_default();
    if !noise.is_empty() {
        game.sound_simple(id, &noise)?;
    }
    let owned = game.entity(id).map(|entity| entity.actor.clone());
    let Some(owned) = owned else {
        return Ok(());
    };
    if game.host.check_client(&owned).is_none() {
        return Ok(());
    }
    let origin = game.body(id)?.origin;
    let (dest1, dest2, plane, color, count) = game
        .entity(id)
        .map(|entity| {
            (
                entity.dest1,
                entity.dest2,
                entity.number("particle_plane"),
                entity.number("color"),
                entity.count,
            )
        })
        .unwrap_or((ZERO, ZERO, 0.0, 0.0, 0.0));
    let start = vadd(dest1, origin);
    let end = vadd(dest2, origin);
    let color = color as i32;
    let count = count as i32;
    if plane == 0.0 {
        let mut x = start.x;
        while x <= end.x {
            let mut z = start.z;
            while z <= end.z {
                game.host.emit(Q1Event::Particles {
                    origin: Vec3 { x, y: start.y, z },
                    direction: ZERO,
                    color,
                    count,
                });
                z += 16.0;
            }
            x += 16.0;
        }
    } else if plane == 1.0 {
        let mut y = start.y;
        while y < end.y {
            let mut z = start.z;
            while z < end.z {
                game.host.emit(Q1Event::Particles {
                    origin: Vec3 { x: start.x, y, z },
                    direction: ZERO,
                    color,
                    count,
                });
                z += 16.0;
            }
            y += 16.0;
        }
    } else {
        let mut x = start.x;
        while x < end.x {
            let mut y = start.y;
            while y < end.y {
                game.host.emit(Q1Event::Particles {
                    origin: Vec3 { x, y, z: start.z },
                    direction: ZERO,
                    color,
                    count,
                });
                y += 16.0;
            }
            x += 16.0;
        }
    }
    Ok(())
}

/// Burn whoever touches an active field.
fn field_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let (damage, ltime, attack_finished) = game
        .entity(id)
        .map(|entity| (entity.damage, entity.number("ltime"), entity.attack_finished))
        .unwrap_or((0.0, 0.0, 0.0));
    if damage == 0.0 || game.time > ltime || game.time < attack_finished {
        return Ok(());
    }
    let time = game.time;
    game.update_entity(id, |entity| entity.attack_finished = time + 0.5)?;
    let id_copy = id.clone();
    let other = other.clone();
    game.damage(
        &other,
        Some(&id_copy),
        Some(&id_copy),
        damage,
        &crate::q1::foundation::entity_services::Q1DamageParams::default(),
    );
    Ok(())
}

/// Spawn a `func_particlefield`.
fn spawn_particlefield(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let bounds = game.body(id)?.bounds;
    let origin = vscale(vadd(bounds.min, bounds.max), 0.5);
    let size = vsub(
        vsub(bounds.max, bounds.min),
        Vec3 {
            x: 16.0,
            y: 16.0,
            z: 16.0,
        },
    );
    let mut dest1 = vsub(vadd(bounds.min, Vec3 { x: 8.0, y: 8.0, z: 8.0 }), origin);
    let dest2 = vsub(vadd(bounds.max, Vec3 { x: 7.9, y: 7.9, z: 7.9 }), origin);
    let plane = if size.x > size.z && size.y > size.z {
        dest1.z = (dest1.z + dest2.z) / 2.0;
        2.0
    } else if size.x <= size.z && size.y > size.x {
        dest1.x = (dest1.x + dest2.x) / 2.0;
        1.0
    } else {
        dest1.y = (dest1.y + dest2.y) / 2.0;
        0.0
    };
    let use_name = game.named.use_callback("hip:particlefield")?;
    let touch_name = game.named.touch("hip:particlefield")?;
    game.update_entity(id, |entity| {
        entity.dest1 = dest1;
        entity.dest2 = dest2;
        number(entity, "particle_plane", plane);
        entity.model.clear();
        entity.solid = Q1Solid::None;
        entity.movement = Q1MoveType::None;
        if entity.count == 0.0 {
            entity.count = 2.0;
        }
        if entity.number("color") == 0.0 {
            number(entity, "color", 192.0);
        }
        entity.use_callback = Some(use_name);
        entity.touch = Some(touch_name);
    })?;
    game.set_origin(id, origin)
}

/// Toggle a wall in or out of the world (`hip:togglewall` use).
fn togglewall_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let active = game
        .entity(id)
        .map(|entity| entity.number("toggle_state"))
        .unwrap_or(0.0)
        == 0.0;
    game.update_entity(id, |entity| {
        number(entity, "toggle_state", if active { 1.0 } else { 0.0 })
    })?;
    let displacement = if active { -8000.0 } else { 8000.0 };
    let origin = game.body(id)?.origin;
    game.set_origin(
        id,
        vadd(
            origin,
            Vec3 {
                x: displacement,
                y: displacement,
                z: displacement,
            },
        ),
    )?;
    let noise = game
        .entity(id)
        .map(|entity| entity.text(if active { "noise1" } else { "noise" }))
        .unwrap_or_default();
    game.sound_simple(id, &noise)
}

/// Burn whoever touches a damaging toggle wall.
fn togglewall_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let (damage, attack_finished) = game
        .entity(id)
        .map(|entity| (entity.damage, entity.attack_finished))
        .unwrap_or((0.0, 0.0));
    if damage == 0.0 || game.time < attack_finished {
        return Ok(());
    }
    let time = game.time;
    game.update_entity(id, |entity| entity.attack_finished = time + 0.5)?;
    let id_copy = id.clone();
    let other = other.clone();
    game.damage(
        &other,
        Some(&id_copy),
        Some(&id_copy),
        damage,
        &crate::q1::foundation::entity_services::Q1DamageParams::default(),
    );
    Ok(())
}

/// Spawn a `func_togglewall`.
fn spawn_togglewall(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let use_name = game.named.use_callback("hip:togglewall")?;
    let touch_name = game.named.touch("hip:togglewall")?;
    game.update_entity(id, |entity| {
        entity.movement = Q1MoveType::Push;
        entity.solid = Q1Solid::Bsp;
        entity.model.clear();
        entity.use_callback = Some(use_name);
        entity.touch = Some(touch_name);
        if entity.text("noise").is_empty() {
            entity.fields.insert("noise".to_string(), "misc/null.wav".to_string());
        }
        if entity.text("noise1").is_empty() {
            entity.fields.insert("noise1".to_string(), "misc/null.wav".to_string());
        }
    })?;
    let spawnflags = game.entity(id).map(|entity| entity.spawnflags).unwrap_or(0);
    if spawnflags & 1 != 0 {
        game.update_entity(id, |entity| number(entity, "toggle_state", 0.0))?;
        let origin = game.body(id)?.origin;
        return game.set_origin(
            id,
            vadd(
                origin,
                Vec3 {
                    x: 8000.0,
                    y: 8000.0,
                    z: 8000.0,
                },
            ),
        );
    }
    game.update_entity(id, |entity| number(entity, "toggle_state", 1.0))?;
    let noise1 = game.entity(id).map(|entity| entity.text("noise1")).unwrap_or_default();
    game.sound_simple(id, &noise1)
}

/// Spawn a `wallsprite`.
fn spawn_wallsprite(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| {
        if entity.model.is_empty() {
            entity.model = "progs/s_blood1.spr".to_string();
        }
    })?;
    let model = game.entity(id).map(|entity| entity.model.clone()).unwrap_or_default();
    game.precache_model(&model)?;
    game.update_entity(id, |entity| {
        entity.solid = Q1Solid::None;
        entity.movement = Q1MoveType::None;
    })?;
    let mut angles = game.body(id)?.angles;
    if angles.x == 0.0 && angles.y == -1.0 && angles.z == 0.0 {
        angles = Vec3 {
            x: -90.0,
            y: 0.0,
            z: 0.0,
        };
    } else if angles.x == 0.0 && angles.y == -2.0 && angles.z == 0.0 {
        angles = Vec3 {
            x: 90.0,
            y: 0.0,
            z: 0.0,
        };
    }
    let origin = game.body(id)?.origin;
    game.set_body(
        id,
        &BodyPatch {
            angles: Some(angles),
            origin: Some(vsub(origin, vscale(vectors(angles).forward, 0.2))),
            ..Default::default()
        },
    )?;
    make_static(game, id)
}

/// Register Hipnotic particle entities (`registerHipnoticParticles`).
pub fn register_hipnotic_particles(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        "hip:particlefield",
        Q1CallbackHandlers {
            use_callback: Some(field_use),
            touch: Some(field_touch),
            ..Default::default()
        },
    )?;
    game.register_spawn("func_particlefield", spawn_particlefield)?;
    game.named.register(
        "hip:togglewall",
        Q1CallbackHandlers {
            use_callback: Some(togglewall_use),
            touch: Some(togglewall_touch),
            ..Default::default()
        },
    )?;
    game.register_spawn("func_togglewall", spawn_togglewall)?;
    game.register_spawn("wallsprite", spawn_wallsprite)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::missionpacks::types::{test_game, test_game_with_events};

    #[test]
    fn particlefield_gates_on_counter_then_arms() {
        let mut game = test_game();
        register_hipnotic_particles(&mut game).expect("register");
        let id = game.create("func_particlefield", None, None).expect("field");
        game.set_body(
            &id,
            &BodyPatch {
                bounds: Some(qa_core::math::Bounds {
                    min: Vec3 {
                        x: -32.0,
                        y: -32.0,
                        z: -32.0,
                    },
                    max: Vec3 {
                        x: 32.0,
                        y: 32.0,
                        z: 32.0,
                    },
                }),
                ..Default::default()
            },
        )
        .expect("bounds");
        game.update_entity(&id, |entity| {
            entity.spawnflags = 1;
            number(entity, "cnt", 5.0);
        })
        .expect("flags");
        game.spawn_entity(&id, None).expect("spawn");
        game.invoke_use(&id, "hip:particlefield", None, None).expect("gated");
        assert_eq!(game.entity(&id).expect("field").number("ltime"), 0.0);
        game.update_entity(&id, |entity| number(entity, "cnt", 0.0))
            .expect("cnt");
        let watch = id.clone();
        game.host.check_client = Box::new(move |_| Some(watch.clone()));
        game.invoke_use(&id, "hip:particlefield", None, None).expect("fire");
        assert_eq!(game.entity(&id).expect("field").number("ltime"), game.time + 0.25);
    }

    #[test]
    fn togglewall_use_flips_wall_out_of_world() {
        let mut game = test_game();
        register_hipnotic_particles(&mut game).expect("register");
        let id = game.create("func_togglewall", None, None).expect("wall");
        game.spawn_entity(&id, None).expect("spawn");
        assert_eq!(game.entity(&id).expect("wall").number("toggle_state"), 1.0);
        game.invoke_use(&id, "hip:togglewall", None, None).expect("use");
        assert_eq!(game.entity(&id).expect("wall").number("toggle_state"), 0.0);
        assert_eq!(
            game.body(&id).expect("body").origin,
            Vec3 {
                x: 8000.0,
                y: 8000.0,
                z: 8000.0
            }
        );
    }

    #[test]
    fn wallsprite_remaps_vertical_angles() {
        let (mut game, events) = test_game_with_events();
        register_hipnotic_particles(&mut game).expect("register");
        let id = game.create("wallsprite", None, None).expect("sprite");
        game.set_body(
            &id,
            &BodyPatch {
                angles: Some(Vec3 {
                    x: 0.0,
                    y: -1.0,
                    z: 0.0,
                }),
                ..Default::default()
            },
        )
        .expect("angles");
        game.spawn_entity(&id, None).expect("spawn");
        assert!(game.entity(&id).is_none());
        let emission = events
            .borrow()
            .events
            .iter()
            .find_map(|event| match event {
                Q1Event::StaticModel { path, angles, .. } => Some((path.clone(), *angles)),
                _ => None,
            })
            .expect("static model");
        assert_eq!(emission.0, "progs/s_blood1.spr");
        assert_eq!(
            emission.1,
            Vec3 {
                x: -90.0,
                y: 0.0,
                z: 0.0
            }
        );
    }
}
