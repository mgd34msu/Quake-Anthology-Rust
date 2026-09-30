//! Rogue pendulum blades (`src/content/q1/missionpacks/world/rogue-pendulum.ts`).
//!
//! pendulum.qc source animation boxes.

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

use crate::q1::foundation::callbacks::{Q1ActionHandler, Q1CallbackHandlers};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::{BodyPatch, TouchSurface};
use crate::q1::foundation::types::{Q1Event, Q1Solid, Q1SoundChannel, ZERO};
use crate::q1::Q1Error;

use super::common::{later, number};

/// One pendulum swing frame (`swings` entry).
struct PendSwing {
    /// Model frame.
    frame: i32,
    /// Bounds minimum along the swing axis.
    min: f32,
    /// Bounds maximum along the swing axis.
    max: f32,
    /// Bounds bottom.
    low: f32,
    /// Bounds top.
    high: f32,
    /// Step delay in seconds.
    delay: f64,
    /// Impact velocity flag.
    impact: i32,
    /// Whether the swing whooshes.
    sound: bool,
}

/// Pendulum swing cycle (`swings`), including the donor's duplicated
/// thirteenth row and return-trip bounds quirks.
const SWINGS: [PendSwing; 26] = [
    PendSwing {
        frame: 0,
        min: -176.0,
        max: -120.0,
        low: 48.0,
        high: 128.0,
        delay: 0.17,
        impact: 1,
        sound: false,
    },
    PendSwing {
        frame: 1,
        min: -172.0,
        max: -112.0,
        low: 12.0,
        high: 88.0,
        delay: 0.15,
        impact: 0,
        sound: false,
    },
    PendSwing {
        frame: 2,
        min: -160.0,
        max: -96.0,
        low: -22.0,
        high: 50.0,
        delay: 0.13,
        impact: 0,
        sound: false,
    },
    PendSwing {
        frame: 3,
        min: -138.0,
        max: -70.0,
        low: -51.0,
        high: 17.0,
        delay: 0.11,
        impact: 0,
        sound: true,
    },
    PendSwing {
        frame: 4,
        min: -110.0,
        max: -38.0,
        low: -72.0,
        high: -8.0,
        delay: 0.09,
        impact: 0,
        sound: false,
    },
    PendSwing {
        frame: 5,
        min: -76.0,
        max: 0.0,
        low: -83.0,
        high: -23.0,
        delay: 0.07,
        impact: 0,
        sound: false,
    },
    PendSwing {
        frame: 6,
        min: -40.0,
        max: 40.0,
        low: -88.0,
        high: -32.0,
        delay: 0.05,
        impact: 0,
        sound: false,
    },
    PendSwing {
        frame: 7,
        min: 0.0,
        max: 76.0,
        low: -83.0,
        high: -23.0,
        delay: 0.07,
        impact: 0,
        sound: false,
    },
    PendSwing {
        frame: 8,
        min: 38.0,
        max: 100.0,
        low: -72.0,
        high: -8.0,
        delay: 0.09,
        impact: 0,
        sound: false,
    },
    PendSwing {
        frame: 9,
        min: 70.0,
        max: 138.0,
        low: -51.0,
        high: 17.0,
        delay: 0.11,
        impact: 0,
        sound: false,
    },
    PendSwing {
        frame: 10,
        min: 96.0,
        max: 160.0,
        low: -22.0,
        high: 50.0,
        delay: 0.13,
        impact: 0,
        sound: false,
    },
    PendSwing {
        frame: 11,
        min: 112.0,
        max: 172.0,
        low: 12.0,
        high: 88.0,
        delay: 0.15,
        impact: 0,
        sound: false,
    },
    PendSwing {
        frame: 12,
        min: 120.0,
        max: 176.0,
        low: 48.0,
        high: 128.0,
        delay: 0.17,
        impact: 0,
        sound: false,
    },
    PendSwing {
        frame: 12,
        min: 120.0,
        max: 176.0,
        low: 48.0,
        high: 128.0,
        delay: 0.17,
        impact: 0,
        sound: false,
    },
    PendSwing {
        frame: 11,
        min: 112.0,
        max: 172.0,
        low: 12.0,
        high: 88.0,
        delay: 0.15,
        impact: -1,
        sound: false,
    },
    PendSwing {
        frame: 10,
        min: 96.0,
        max: 160.0,
        low: -22.0,
        high: 50.0,
        delay: 0.13,
        impact: 0,
        sound: false,
    },
    PendSwing {
        frame: 9,
        min: 70.0,
        max: 138.0,
        low: -51.0,
        high: 17.0,
        delay: 0.11,
        impact: 0,
        sound: true,
    },
    PendSwing {
        frame: 8,
        min: 38.0,
        max: 100.0,
        low: -72.0,
        high: -8.0,
        delay: 0.09,
        impact: 0,
        sound: false,
    },
    PendSwing {
        frame: 7,
        min: 0.0,
        max: 76.0,
        low: -83.0,
        high: -23.0,
        delay: 0.07,
        impact: 0,
        sound: false,
    },
    PendSwing {
        frame: 6,
        min: -40.0,
        max: 40.0,
        low: -88.0,
        high: -32.0,
        delay: 0.05,
        impact: 0,
        sound: false,
    },
    PendSwing {
        frame: 5,
        min: -76.0,
        max: 0.0,
        low: -83.0,
        high: -23.0,
        delay: 0.07,
        impact: 0,
        sound: false,
    },
    PendSwing {
        frame: 4,
        min: -110.0,
        max: -28.0,
        low: -72.0,
        high: -8.0,
        delay: 0.09,
        impact: 0,
        sound: false,
    },
    PendSwing {
        frame: 3,
        min: -172.0,
        max: -70.0,
        low: -51.0,
        high: 17.0,
        delay: 0.11,
        impact: 0,
        sound: false,
    },
    PendSwing {
        frame: 2,
        min: -160.0,
        max: -96.0,
        low: -22.0,
        high: 50.0,
        delay: 0.13,
        impact: 0,
        sound: false,
    },
    PendSwing {
        frame: 1,
        min: -172.0,
        max: -112.0,
        low: 12.0,
        high: 88.0,
        delay: 0.15,
        impact: 0,
        sound: false,
    },
    PendSwing {
        frame: 0,
        min: -176.0,
        max: -120.0,
        low: 48.0,
        high: 128.0,
        delay: 0.17,
        impact: 0,
        sound: false,
    },
];

/// Run one swing step, scheduling the next.
fn pend_step(game: &mut Q1EntityServices, id: &ActorId, index: usize) -> Result<(), Q1Error> {
    let row = &SWINGS[index];
    game.update_entity(id, |entity| entity.frame = row.frame)?;
    if index != 13 {
        let spawnflags = game.entity(id).map(|entity| entity.spawnflags).unwrap_or(0);
        game.set_bounds(
            id,
            if spawnflags & 2 != 0 {
                Bounds {
                    min: Vec3 {
                        x: -8.0,
                        y: row.min,
                        z: row.low,
                    },
                    max: Vec3 {
                        x: 8.0,
                        y: row.max,
                        z: row.high,
                    },
                }
            } else {
                Bounds {
                    min: Vec3 {
                        x: row.min,
                        y: -8.0,
                        z: row.low,
                    },
                    max: Vec3 {
                        x: row.max,
                        y: 8.0,
                        z: row.high,
                    },
                }
            },
        )?;
    }
    if row.impact != 0 {
        if let Some(world) = game.world.clone() {
            let impact = f64::from(row.impact);
            game.update_entity(&world, |world| number(world, "rogue:impactVelocity", impact))?;
        }
    }
    if row.sound {
        game.host.emit(Q1Event::Sound {
            origin: None,
            actor: id.clone(),
            path: "pendulum/swing.wav".to_string(),
            channel: Q1SoundChannel::Auto,
            attenuation: 1.0,
            volume: 0.5,
        });
    }
    later(
        game,
        id,
        row.delay,
        &format!("rogue:pend_swing{}", (index + 1) % SWINGS.len() + 1),
    )
}

macro_rules! pend_swing {
    ($name:ident, $index:expr) => {
        fn $name(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
            pend_step(game, id, $index)
        }
    };
}

pend_swing!(pend_swing_1, 0);
pend_swing!(pend_swing_2, 1);
pend_swing!(pend_swing_3, 2);
pend_swing!(pend_swing_4, 3);
pend_swing!(pend_swing_5, 4);
pend_swing!(pend_swing_6, 5);
pend_swing!(pend_swing_7, 6);
pend_swing!(pend_swing_8, 7);
pend_swing!(pend_swing_9, 8);
pend_swing!(pend_swing_10, 9);
pend_swing!(pend_swing_11, 10);
pend_swing!(pend_swing_12, 11);
pend_swing!(pend_swing_13, 12);
pend_swing!(pend_swing_14, 13);
pend_swing!(pend_swing_15, 14);
pend_swing!(pend_swing_16, 15);
pend_swing!(pend_swing_17, 16);
pend_swing!(pend_swing_18, 17);
pend_swing!(pend_swing_19, 18);
pend_swing!(pend_swing_20, 19);
pend_swing!(pend_swing_21, 20);
pend_swing!(pend_swing_22, 21);
pend_swing!(pend_swing_23, 22);
pend_swing!(pend_swing_24, 23);
pend_swing!(pend_swing_25, 24);
pend_swing!(pend_swing_26, 25);

/// Slice and fling whoever touches the blade.
fn pend_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    if game.health(other) < 1.0
        || !game
            .host
            .combat
            .read(other)
            .is_some_and(|combat| combat.can_take_damage)
    {
        return Ok(());
    }
    if game.entity(id).map(|entity| entity.attack_finished).unwrap_or(0.0) < game.time {
        game.sound_simple(id, "pendulum/hit.wav")?;
        let time = game.time;
        game.update_entity(id, |entity| entity.attack_finished = time + 1.0)?;
    }
    let damage = game
        .entity(id)
        .map(|entity| entity.number("currentammo"))
        .unwrap_or(0.0);
    let id_copy = id.clone();
    let other_copy = other.clone();
    game.damage(
        &other_copy,
        Some(&id_copy),
        Some(&id_copy),
        damage,
        &crate::q1::foundation::entity_services::Q1DamageParams::default(),
    );
    let body = game.host.bodies.read(other);
    let owned = game.host.actors.resolve_owned(other);
    let (Some(body), Some(owned)) = (body, owned) else {
        return Ok(());
    };
    let impact = game
        .world
        .as_ref()
        .and_then(|world| game.entity(world))
        .map(|world| world.number("rogue:impactVelocity"))
        .unwrap_or(0.0);
    let spawnflags = game.entity(id).map(|entity| entity.spawnflags).unwrap_or(0);
    let mut next = body.clone();
    next.velocity = if spawnflags & 2 != 0 {
        Vec3 {
            x: body.velocity.x,
            y: (impact * -250.0) as f32,
            z: 200.0,
        }
    } else {
        Vec3 {
            x: (impact * 250.0) as f32,
            y: body.velocity.y,
            z: 200.0,
        }
    };
    game.host.bodies.write(&owned, &next)?;
    game.effect(
        crate::q1::foundation::types::Q1Effect::MeatSpray,
        body.origin,
        Some(other),
        1,
    );
    Ok(())
}

/// Start swinging from a use dispatch.
fn pend_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let delay = game.entity(id).map(|entity| entity.delay).unwrap_or(0.0);
    later(game, id, delay, "rogue:pend_swing1")
}

/// Spawn a `pendulum`.
fn spawn_pendulum(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| {
        entity.model = "progs/pendulum.mdl".to_string();
        if entity.spawnflags == 0 {
            entity.spawnflags = 2;
        }
    })?;
    let spawnflags = game.entity(id).map(|entity| entity.spawnflags).unwrap_or(0);
    if spawnflags & 3 == 0 {
        return Err(crate::q1::q1_error("Unimplemented Pendulum Type (pendulum.qc)"));
    }
    game.set_body(
        id,
        &BodyPatch {
            angles: Some(if spawnflags & 2 != 0 {
                ZERO
            } else {
                Vec3 {
                    x: 0.0,
                    y: 270.0,
                    z: 0.0,
                }
            }),
            bounds: Some(if spawnflags & 2 != 0 {
                Bounds {
                    min: Vec3 {
                        x: -8.0,
                        y: -24.0,
                        z: -100.0,
                    },
                    max: Vec3 {
                        x: 8.0,
                        y: 24.0,
                        z: 100.0,
                    },
                }
            } else {
                Bounds {
                    min: Vec3 {
                        x: -24.0,
                        y: -8.0,
                        z: -100.0,
                    },
                    max: Vec3 {
                        x: 24.0,
                        y: 8.0,
                        z: 100.0,
                    },
                }
            }),
            ..Default::default()
        },
    )?;
    game.update_entity(id, |entity| {
        if entity.number("currentammo") == 0.0 {
            number(entity, "currentammo", 5.0);
        }
        if entity.delay == 0.0 {
            entity.delay = 1.0;
        }
        entity.solid = Q1Solid::Trigger;
    })?;
    game.set_damageable(id, false)?;
    let touch_name = game.named.touch("rogue:pend_touch")?;
    game.update_entity(id, |entity| entity.touch = Some(touch_name))?;
    if let Some(world) = game.world.clone() {
        game.update_entity(&world, |world| number(world, "rogue:impactVelocity", 0.0))?;
    }
    if spawnflags & 8 != 0 {
        let use_name = game.named.use_callback("rogue:pend_use")?;
        return game.update_entity(id, |entity| entity.use_callback = Some(use_name));
    }
    let delay = game.entity(id).map(|entity| entity.delay).unwrap_or(0.0);
    later(game, id, delay, "rogue:pend_swing1")
}

/// Register Rogue pendulum entities (`registerRoguePendulum`).
pub fn register_rogue_pendulum(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    for (name, action) in [
        ("rogue:pend_swing1", pend_swing_1 as Q1ActionHandler),
        ("rogue:pend_swing2", pend_swing_2 as Q1ActionHandler),
        ("rogue:pend_swing3", pend_swing_3 as Q1ActionHandler),
        ("rogue:pend_swing4", pend_swing_4 as Q1ActionHandler),
        ("rogue:pend_swing5", pend_swing_5 as Q1ActionHandler),
        ("rogue:pend_swing6", pend_swing_6 as Q1ActionHandler),
        ("rogue:pend_swing7", pend_swing_7 as Q1ActionHandler),
        ("rogue:pend_swing8", pend_swing_8 as Q1ActionHandler),
        ("rogue:pend_swing9", pend_swing_9 as Q1ActionHandler),
        ("rogue:pend_swing10", pend_swing_10 as Q1ActionHandler),
        ("rogue:pend_swing11", pend_swing_11 as Q1ActionHandler),
        ("rogue:pend_swing12", pend_swing_12 as Q1ActionHandler),
        ("rogue:pend_swing13", pend_swing_13 as Q1ActionHandler),
        ("rogue:pend_swing14", pend_swing_14 as Q1ActionHandler),
        ("rogue:pend_swing15", pend_swing_15 as Q1ActionHandler),
        ("rogue:pend_swing16", pend_swing_16 as Q1ActionHandler),
        ("rogue:pend_swing17", pend_swing_17 as Q1ActionHandler),
        ("rogue:pend_swing18", pend_swing_18 as Q1ActionHandler),
        ("rogue:pend_swing19", pend_swing_19 as Q1ActionHandler),
        ("rogue:pend_swing20", pend_swing_20 as Q1ActionHandler),
        ("rogue:pend_swing21", pend_swing_21 as Q1ActionHandler),
        ("rogue:pend_swing22", pend_swing_22 as Q1ActionHandler),
        ("rogue:pend_swing23", pend_swing_23 as Q1ActionHandler),
        ("rogue:pend_swing24", pend_swing_24 as Q1ActionHandler),
        ("rogue:pend_swing25", pend_swing_25 as Q1ActionHandler),
        ("rogue:pend_swing26", pend_swing_26 as Q1ActionHandler),
    ] {
        game.named.register(
            name,
            Q1CallbackHandlers {
                action: Some(action),
                ..Default::default()
            },
        )?;
    }
    game.named.register(
        "rogue:pend_touch",
        Q1CallbackHandlers {
            touch: Some(pend_touch),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:pend_use",
        Q1CallbackHandlers {
            use_callback: Some(pend_use),
            ..Default::default()
        },
    )?;
    game.register_spawn("pendulum", spawn_pendulum)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::missionpacks::types::test_game;

    #[test]
    fn pendulum_spawn_starts_swing_cycle() {
        let mut game = test_game();
        register_rogue_pendulum(&mut game).expect("register");
        let id = game.create("pendulum", None, None).expect("pendulum");
        game.spawn_entity(&id, None).expect("spawn");
        assert_eq!(
            game.entity(&id).expect("pendulum").think.as_deref(),
            Some("rogue:pend_swing1")
        );
        game.invoke_action(&id, "rogue:pend_swing1").expect("swing");
        assert_eq!(game.entity(&id).expect("pendulum").frame, 0);
        let bounds = game.body(&id).expect("body").bounds;
        assert_eq!(bounds.min.y, -176.0);
        assert_eq!(bounds.max.y, -120.0);
        assert_eq!(
            game.entity(&id).expect("pendulum").think.as_deref(),
            Some("rogue:pend_swing2")
        );
    }

    #[test]
    fn duplicated_swing_skips_bounds() {
        let mut game = test_game();
        register_rogue_pendulum(&mut game).expect("register");
        let id = game.create("pendulum", None, None).expect("pendulum");
        game.spawn_entity(&id, None).expect("spawn");
        let before = game.body(&id).expect("body").bounds;
        game.invoke_action(&id, "rogue:pend_swing14").expect("swing");
        assert_eq!(game.entity(&id).expect("pendulum").frame, 12);
        assert_eq!(game.body(&id).expect("body").bounds, before);
        assert_eq!(
            game.entity(&id).expect("pendulum").think.as_deref(),
            Some("rogue:pend_swing15")
        );
    }

    #[test]
    fn pend_touch_flings_victims_upward() {
        let mut game = test_game();
        register_rogue_pendulum(&mut game).expect("register");
        let id = game.create("pendulum", None, None).expect("pendulum");
        game.spawn_entity(&id, None).expect("spawn");
        let victim = game.create("player", None, None).expect("victim");
        game.set_health(&victim, 100.0).expect("health");
        game.set_damageable(&victim, true).expect("damageable");
        game.time = 0.5;
        game.invoke_touch(&id, &victim, None, None).expect("touch");
        let velocity = game.host.bodies.read(&victim).expect("body").velocity;
        assert_eq!(f64::from(velocity.z), 200.0);
        assert!(game.entity(&id).expect("pendulum").attack_finished > 0.0);
    }
}
