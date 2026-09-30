//! Dormant spike mine spawn (`src/content/q1/missionpacks/monsters/spikemine.ts`).

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::types::{Q1MoveType, Q1Solid};
use crate::q1::Q1Error;

/// Register the dormant `monster_spikemine` spawn (`registerDormantSpikemine`).
/// The active `trap_spike_mine` lives in `hipitems.qc`.
pub fn register_dormant_spikemine(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.register_spawn("monster_spikemine", dormant_spikemine_spawn)
}

fn dormant_spikemine_spawn(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if game.options().deathmatch != 0 {
        return game.remove(id);
    }
    game.update_entity(id, |entity| {
        entity.solid = Q1Solid::Slidebox;
        entity.movement = Q1MoveType::Step;
        entity.model = String::from("progs/demon.mdl");
    })?;
    game.set_bounds(
        id,
        Bounds {
            min: Vec3 {
                x: -32.0,
                y: -32.0,
                z: -24.0,
            },
            max: Vec3 {
                x: 32.0,
                y: 32.0,
                z: 64.0,
            },
        },
    )?;
    game.link(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::missionpacks::types::test_game;

    #[test]
    fn dormant_spikemine_links() {
        let mut game = test_game();
        register_dormant_spikemine(&mut game).expect("register");
        let id = game.create("monster_spikemine", None, None).expect("create");
        dormant_spikemine_spawn(&mut game, &id).expect("spawn");
        let entity = game.entity(&id).expect("entity").clone();
        assert_eq!(entity.model, "progs/demon.mdl");
        assert_eq!(entity.solid, Q1Solid::Slidebox);
    }
}
