//! Quake III base/game: use participant.
//!
//! Donor provenance: `src/content/q3/base/game/use-participant.ts`.

use qa_core::identity::ActorId;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::state::*;
use crate::q3::base::game::state::{failure, EntityPool, Q3Driver, Q3GameError};

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

#[cfg(test)]
mod tests {
    use super::*;

    use crate::q3::base::game::state::test_support::*;

    use crate::q3::base::shared::definitions::Product;

    #[test]
    fn use_participant_branches() {
        let owner = test_owner();
        let mut driver = StubDriver::new(&owner, Product::Baseq3);
        assert!(require_use_participant(None).is_err());
        let slot = driver.pool.spawn_entity().unwrap();
        driver.pool.entities[slot].client = Some(0);
        let native = Participant::Entity(slot);
        assert_eq!(
            use_actor(&driver.pool, &native).unwrap(),
            driver.pool.entities[slot].actor.clone()
        );
        assert_eq!(use_client(&mut driver, &native).unwrap(), Some(slot));
        driver.pool.entities[slot].client = None;
        assert_eq!(use_client(&mut driver, &native).unwrap(), None);
        let shared = owner.actor(9000, 1);
        driver.live.insert(shared.clone(), true);
        driver.players.insert(shared.clone(), true);
        driver.native_map.insert(shared.clone(), slot);
        driver.pool.entities[slot].client = Some(0);
        assert_eq!(
            use_client(&mut driver, &Participant::SharedActor(shared.clone())).unwrap(),
            Some(slot)
        );
        driver.players.insert(shared.clone(), false);
        assert_eq!(
            use_client(&mut driver, &Participant::SharedActor(shared)).unwrap(),
            None
        );
    }
}
