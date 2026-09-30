//! Q1 mg3 heavy projectiles (`src/content/q1/addons/monsters/heavy/projectiles.ts`).
//!
//! MG3 launch_spike and LightningDamage source operations.
//! GPL-2.0-or-later.

use qa_core::identity::{same_actor, ActorId};
use qa_core::math::Vec3;

use crate::q1::base::projectiles::{launch_spike, SpikeKind};
use crate::q1::foundation::callbacks::{Q1CallbackHandlers, Q1TouchHandler};
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::host::Q1Contents;
use crate::q1::foundation::types::{vadd, vscale, vsub, Q1Effect, Q1Event, Q1Solid, Q1TraceRequest, POINT, ZERO};
use crate::q1::{q1_error, Q1Error};

use super::runtime::HEAVY_PREFIX;

/// Launch a heavy spike (`heavySpike`).
pub fn heavy_spike(
    game: &mut Q1EntityServices,
    owner: &ActorId,
    origin: Vec3,
    velocity: Vec3,
) -> Result<ActorId, Q1Error> {
    let missile = launch_spike(game, Some(owner), origin, velocity, SpikeKind::Spike)?;
    let touch = game.named.touch(&format!("{HEAVY_PREFIX}:spike_touch"))?;
    game.update_entity(&missile, |entity| {
        entity.classname = String::from("knightspike");
        entity.touch = Some(touch);
    })?;
    Ok(missile)
}

fn heavy_spike_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    let entity = game
        .entity_ref(id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    if entity.owner.as_ref().is_some_and(|owner| same_actor(other, owner))
        || game
            .entity_ref(other)
            .is_some_and(|other| other.solid == Q1Solid::Trigger)
    {
        return Ok(());
    }
    let origin = game.body(id).map(|body| body.origin)?;
    if game.host.contents(origin) == Q1Contents::Sky {
        return game.remove(id);
    }
    if game
        .host
        .combat
        .read(other)
        .is_some_and(|combat| combat.can_take_damage)
    {
        game.effect(Q1Effect::Blood, origin, Some(other), 9);
        game.damage(other, Some(id), entity.owner.as_ref(), 9.0, &Q1DamageParams::default());
    } else {
        game.effect(Q1Effect::KnightSpike, origin, None, 1);
    }
    game.remove(id)
}

/// Register heavy projectile touches (`registerHeavyProjectiles`).
pub fn register_heavy_projectiles(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        &format!("{HEAVY_PREFIX}:spike_touch"),
        Q1CallbackHandlers {
            touch: Some(heavy_spike_touch as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    Ok(())
}

/// Damage actors along a lightning bolt (`heavyLightningDamage`).
pub fn heavy_lightning_damage(
    game: &mut Q1EntityServices,
    actor: &ActorId,
    start: Vec3,
    end: Vec3,
    damage: f64,
) -> Result<(), Q1Error> {
    let delta = vsub(end, start);
    // The donor offsets both lanes by -delta.y.
    let side = Vec3 {
        x: -delta.y * 16.0,
        y: -delta.y * 16.0,
        z: 0.0,
    };
    let mut hit: Vec<Option<ActorId>> = Vec::new();
    for offset in [ZERO, side, vscale(side, -1.0)] {
        let trace = game.host.trace(&Q1TraceRequest {
            start: vadd(start, offset),
            end: vadd(end, offset),
            bounds: POINT,
            ignore: Some(actor.clone()),
            monsters: true,
            missile: false,
        });
        if let Some(target) = trace.actor.clone() {
            if !hit.iter().flatten().any(|previous| same_actor(previous, &target))
                && game
                    .host
                    .combat
                    .read(&target)
                    .is_some_and(|combat| combat.can_take_damage)
            {
                game.host.emit(Q1Event::Particles {
                    origin: trace.end,
                    direction: Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 100.0,
                    },
                    color: 246,
                    count: (damage * 4.0) as i32,
                });
                game.damage(&target, Some(actor), Some(actor), damage, &Q1DamageParams::default());
            }
        }
        hit.push(trace.actor);
    }
    Ok(())
}
