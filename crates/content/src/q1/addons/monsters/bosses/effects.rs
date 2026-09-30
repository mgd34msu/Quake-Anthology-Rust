//! Q1 final-boss effects (`src/content/q1/addons/monsters/bosses/effects.ts`).
//!
//! `boss_final.qc` pain_lightning. GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::q1::addons::context::Q1AddonContext;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::types::{vadd, vscale, Q1BeamStyle, Q1Event, Q1SoundChannel, Q1TraceRequest, POINT};

/// Fire pain lightning from a final boss (`painLightning`).
pub fn pain_lightning(
    _context: &Q1AddonContext,
    game: &mut Q1EntityServices,
    id: &ActorId,
    offset: Vec3,
) -> Result<(), crate::q1::Q1Error> {
    let pitch = (game.host.random() * 180.0 + 180.0) as f32;
    let yaw = (game.host.random() * 360.0) as f32;
    let forward = game
        .make_vectors(Vec3 {
            x: pitch,
            y: yaw,
            z: 0.0,
        })
        .forward;
    let origin = vadd(game.body(id).map(|body| body.origin)?, offset);
    let trace = game.host.trace(&Q1TraceRequest {
        start: origin,
        end: vadd(origin, vscale(forward, 1000.0)),
        bounds: POINT,
        ignore: Some(id.clone()),
        monsters: false,
        missile: false,
    });
    game.sound(id, "misc/power.wav", Q1SoundChannel::Body, 1.0, 1.0)?;
    game.host.emit(Q1Event::Beam {
        style: Q1BeamStyle::Lightning3,
        actor: id.clone(),
        start: origin,
        end: trace.end,
    });
    Ok(())
}
