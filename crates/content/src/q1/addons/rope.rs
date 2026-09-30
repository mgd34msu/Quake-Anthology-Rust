//! Q1 addon ropes (`src/content/q1/addons/rope.ts`).
//!
//! `quakec_mg3/misc_rope.qc`. Copyright (C) 1996-2026 id Software LLC.
//! GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

use crate::q1::addons::context::{add_frame_tick, require_entity, set_addon_vector, Q1AddonContext};
use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::types::{length, vadd, vsub, Q1MoveType, Q1Solid, Q1TraceRequest, POINT};
use crate::q1::{q1_error, Q1Error};

/// Rope frame-tick callback name.
pub const ROPE_TICK: &str = "mg3:rope:tick";

/// Follow the rope segment chain (`next`).
fn next(game: &Q1EntityServices, id: &ActorId) -> Result<Option<ActorId>, Q1Error> {
    let chained = require_entity(game, id)?
        .references
        .get("rope.chain")
        .cloned()
        .flatten();
    Ok(match chained {
        Some(chained) if game.entity_ref(&chained).is_some() => Some(chained),
        _ => None,
    })
}

fn rope_tick(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let origin = require_entity(game, id)?.vector("oldorigin");
    let top = game.host.trace(&Q1TraceRequest {
        start: origin,
        end: vadd(
            origin,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 2048.0,
            },
        ),
        bounds: POINT,
        ignore: Some(id.clone()),
        monsters: false,
        missile: false,
    });
    let bottom = game.host.trace(&Q1TraceRequest {
        start: origin,
        end: vadd(
            origin,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: -2048.0,
            },
        ),
        bounds: POINT,
        ignore: Some(id.clone()),
        monsters: false,
        missile: false,
    });
    let mut segments = f64::from(length(vsub(bottom.end, top.end)) / 32.0).floor() as i32;
    let models = (segments / 4).clamp(0, 32);
    while models < require_entity(game, id)?.count as i32 {
        let first = next(game, id)?;
        let following = match &first {
            Some(first) => next(game, first)?,
            None => None,
        };
        if first.as_ref().is_some_and(|first| {
            game.entity_ref(first)
                .is_some_and(|entity| entity.classname == "misc_rope_segment")
        }) {
            if let Some(first) = &first {
                game.remove(first)?;
            }
        }
        game.update_entity(id, |entity| {
            entity.references.insert(String::from("rope.chain"), following);
            entity.count -= 1.0;
        })?;
    }
    while models > require_entity(game, id)?.count as i32 {
        let segment = game.create("misc_rope_segment", None, None)?;
        let skin = require_entity(game, id)?.skin;
        game.update_entity(&segment, |entity| {
            entity.model = String::from("progs/ropex.mdl");
            entity.frame = 3;
            entity.skin = skin;
            entity.solid = Q1Solid::None;
            entity.movement = Q1MoveType::None;
        })?;
        game.set_damageable(&segment, false)?;
        // The source sets the parent bounds here, then sets its final
        // bounds below.
        game.set_bounds(
            id,
            Bounds {
                min: Vec3 {
                    x: -4.0,
                    y: -4.0,
                    z: 0.0,
                },
                max: Vec3 {
                    x: 4.0,
                    y: 4.0,
                    z: 128.0,
                },
            },
        )?;
        let chained = require_entity(game, id)?
            .references
            .get("rope.chain")
            .cloned()
            .flatten();
        game.update_entity(&segment, |entity| {
            entity.references.insert(String::from("rope.chain"), chained);
        })?;
        game.update_entity(id, |entity| {
            entity.references.insert(String::from("rope.chain"), Some(segment));
            entity.count += 1.0;
        })?;
    }
    let mut segment = next(game, id)?;
    let mut position = bottom.end;
    for _ in 0..models {
        let current = segment
            .clone()
            .ok_or_else(|| q1_error("misc_rope lost its source segment chain"))?;
        game.set_origin(&current, position)?;
        position = vadd(
            position,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 128.0,
            },
        );
        segment = next(game, &current)?;
        segments -= 4;
    }
    game.set_origin(id, position)?;
    game.update_entity(id, |entity| entity.frame = segments)?;
    game.set_bounds(
        id,
        Bounds {
            min: Vec3 {
                x: -4.0,
                y: -4.0,
                z: 0.0,
            },
            max: Vec3 {
                x: 4.0,
                y: 4.0,
                z: 32.0 * (segments + 1) as f32,
            },
        },
    )
}

fn spawn_misc_rope(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let origin = game.body(id)?.origin;
    let skin = require_entity(game, id)?.number("skin") as i32;
    game.update_entity(id, |entity| {
        entity.model = String::from("progs/ropex.mdl");
        entity.skin = skin;
        entity.count = 0.0;
        entity.solid = Q1Solid::None;
        entity.movement = Q1MoveType::None;
    })?;
    game.set_damageable(id, false)?;
    set_addon_vector(game, id, "oldorigin", origin)?;
    game.set_bounds(
        id,
        Bounds {
            min: Vec3 {
                x: -4.0,
                y: -4.0,
                z: 0.0,
            },
            max: Vec3 {
                x: 4.0,
                y: 4.0,
                z: 32.0,
            },
        },
    )?;
    add_frame_tick(game, id, ROPE_TICK)
}

/// Register addon ropes (`registerAddonRopes`).
pub fn register_addon_ropes(_context: &Q1AddonContext, game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        ROPE_TICK,
        Q1CallbackHandlers {
            action: Some(rope_tick),
            ..Default::default()
        },
    )?;
    game.register_spawn("misc_rope", spawn_misc_rope)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::{frame_addons, register_test_addons, Q1AddonProgram};
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::missionpacks::types::test_game;

    fn setup(game: &mut Q1EntityServices, program: Q1AddonProgram) -> (Q1BaseGuard, Q1AddonContext) {
        let guard = Q1BaseGuard::register(game, Q1BaseOptions::default()).expect("base");
        let context = register_test_addons(game, program);
        register_addon_ropes(&context, game).expect("ropes");
        (guard, context)
    }

    #[test]
    fn rope_spawn_marks_frame_tick() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg3);
        let rope = game.create("misc_rope", None, None).expect("rope");
        spawn_misc_rope(&mut game, &rope).expect("spawn");
        let entity = require_entity(&game, &rope).expect("entity");
        assert_eq!(entity.model, "progs/ropex.mdl");
        assert_eq!(entity.count, 0.0);
        assert_eq!(entity.text("addon.frameTick"), ROPE_TICK);
        assert_eq!(entity.vector("oldorigin"), game.body(&rope).expect("body").origin);
    }

    #[test]
    fn rope_tick_builds_segment_chain() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg3);
        let rope = game.create("misc_rope", None, None).expect("rope");
        spawn_misc_rope(&mut game, &rope).expect("spawn");
        // Mock traces never hit, so the 4096-unit span builds the full
        // 32-segment chain and leaves no remainder.
        rope_tick(&mut game, &rope).expect("tick");
        let entity = require_entity(&game, &rope).expect("entity");
        assert_eq!(entity.count, 32.0);
        assert_eq!(entity.frame, 0);
        let mut chained = 0;
        let mut current = next(&game, &rope).expect("chain");
        while let Some(id) = current {
            chained += 1;
            assert_eq!(require_entity(&game, &id).expect("segment").model, "progs/ropex.mdl");
            current = next(&game, &id).expect("chain");
        }
        assert_eq!(chained, 32);
        frame_addons(&mut game, 1.0).expect("frame");
        assert_eq!(require_entity(&game, &rope).expect("entity").count, 32.0);
    }
}
