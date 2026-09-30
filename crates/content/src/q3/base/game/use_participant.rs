//! Quake III base/game: use participant.
//!
//! Donor provenance: `src/content/q3/base/game/use-participant.ts`.

use qa_core::identity::ActorId;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::mirrors_game_state::*;
use crate::q3::base::game::state::*;

// ---------------------------------------------------------------------------
// use-participant.ts: participant helpers
// ---------------------------------------------------------------------------

/// Use-participant services (`UseParticipantServices`).
pub trait UseParticipantServices {
    /// Whether an actor is live.
    fn live(&self, actor: &ActorId) -> bool;
    /// Whether an actor is a player.
    fn is_player(&self, actor: &ActorId) -> bool;
    /// Native slot for an actor.
    fn native(&self, actor: &ActorId) -> Option<usize>;
    /// Emit an actor event.
    fn event(&mut self, actor: &ActorId, event: i32, parameter: i32);
}

/// Actor for a participant (`useActor`).
pub fn use_actor(pool: &dyn EntityPool, participant: &Participant) -> Result<ActorId, Q3GameError> {
    match participant {
        Participant::Entity(slot) => pool.entity(*slot).map(|entity| entity.actor.clone()).ok_or_else(|| {
            failure(format!(
                "entity {slot} does not belong to its entity pool or was replaced"
            ))
        }),
        Participant::SharedActor(actor) => Ok(actor.clone()),
    }
}

/// Require an activator (`requireUseParticipant`).
pub fn require_use_participant(participant: Option<Participant>) -> Result<Participant, Q3GameError> {
    participant.ok_or_else(|| failure("Target handler requires an activator"))
}

/// Client entity for a participant (`useClient`).
pub fn use_client(driver: &mut dyn Q3Driver, participant: &Participant) -> Result<Option<usize>, Q3GameError> {
    match participant {
        Participant::Entity(slot) => {
            let slot = *slot;
            Ok(
                if driver.pool().entity(slot).is_some_and(|entity| entity.client.is_some()) {
                    Some(slot)
                } else {
                    None
                },
            )
        }
        Participant::SharedActor(actor) => {
            if !driver.actor_live(actor) || !driver.actor_is_player(actor) {
                return Ok(None);
            }
            let native = driver.native_slot(actor);
            match native {
                Some(slot) if driver.pool().entity(slot).is_some_and(|entity| entity.client.is_some()) => {
                    Ok(Some(slot))
                }
                _ => Err(failure("Admitted Q3 map player has no native client behavior record")),
            }
        }
    }
}
