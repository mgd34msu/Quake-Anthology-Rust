//! Quake III base/game: ground.
//!
//! Donor provenance: `src/content/q3/base/game/ground.ts`.

use qa_core::identity::ActorId;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::entities::*;
use crate::q3::base::shared::player_state::{ENTITYNUM_NONE, ENTITYNUM_WORLD};
use crate::q3::base::world::ActorTraceHit;

// ---------------------------------------------------------------------------
// Ground support (ground.ts).
// ---------------------------------------------------------------------------

/// Entity number of a ground actor (`groundNumber`).
#[must_use]
pub fn ground_number(ground: Option<&ActorId>, pool: &EntityPool) -> i32 {
    match ground {
        None => ENTITYNUM_NONE,
        Some(actor) => pool
            .native_by_actor(actor)
            .map_or(ENTITYNUM_NONE, |entity| entity.borrow().slot as i32),
    }
}

/// Write ground support identity (`writeGround`).
pub fn write_ground(entity: &EntityRef, ground: Option<ActorId>, pool: &EntityPool) {
    let number = ground_number(ground.as_ref(), pool);
    let mut borrowed = entity.borrow_mut();
    borrowed.r.ground = ground;
    borrowed.s.ground_entity_num = number;
}

/// Record trace-hit ground support (`traceGround`).
pub fn trace_ground(entity: &EntityRef, hit: &ActorTraceHit, pool: &EntityPool) {
    let ground = match hit {
        ActorTraceHit::Actor { actor } => Some(actor.clone()),
        ActorTraceHit::World => Some(pool.at(ENTITYNUM_WORLD).borrow().actor.id.clone()),
        ActorTraceHit::None => None,
    };
    write_ground(entity, ground, pool);
}

/// Drop ground support, keeping `-1` for delayed continuation (`loseGround`).
pub fn lose_ground(entity: &EntityRef) {
    let mut borrowed = entity.borrow_mut();
    borrowed.r.ground = None;
    borrowed.s.ground_entity_num = -1;
}

/// True when an entity rides a support entity (`rides`).
#[must_use]
pub fn rides(entity: &EntityRef, support: &EntityRef) -> bool {
    let support_actor = support.borrow().actor.id.clone();
    entity.borrow().r.ground.as_ref() == Some(&support_actor)
}
