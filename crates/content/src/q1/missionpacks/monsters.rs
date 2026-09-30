//! Q1 mission-pack monsters root (`src/content/q1/missionpacks/monsters`, barrel `index.ts`).
//!
//! Renames against the donor barrel: `registerMissionPackMonsters` is
//! [`register_mission_pack_monsters`], `becomeDecoy` is
//! [`decoy::become_decoy`], and `launchDragonFireball` is
//! [`dragon::launch_dragon_fireball`]. The donor `base` handle has no
//! analogue: monsters reach the base game through free functions.

pub mod armagon;
pub mod charm;
pub mod charmed_base;
pub mod decoy;
pub mod dragon;
pub mod eel;
pub mod gremlin;
pub mod gremlin_ai;
pub mod gremlin_weapons;
pub mod helpers;
pub mod lavaman;
pub mod morph;
pub mod mummy;
pub mod overlord;
pub mod paths;
pub mod runtime;
pub mod scourge;
pub mod spikemine;
pub mod sword;
pub mod tables;
pub mod types;
pub mod wrath;

use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::missionpacks::types::Q1MissionPack;
use crate::q1::Q1Error;

pub use self::decoy::become_decoy;
pub use self::dragon::launch_dragon_fireball;
pub use self::runtime::{MissionMonster, Q1MissionPackMonsters};
pub use self::types::{MissionAction, MissionMonsterHooks, PackMonsterDefinition, PackMonsterFactory};

use self::armagon::armagon_definition;
use self::charmed_base::{hipnotic_army_definition, hipnotic_dog_definition};
use self::decoy::decoy_definition;
use self::dragon::{dragon_definition, register_dragon_corners};
use self::eel::eel_definition;
use self::gremlin::gremlin_definition;
use self::lavaman::lavaman_definition;
use self::morph::morph_definition;
use self::mummy::mummy_definition;
use self::overlord::{overlord_definition, register_overlord_destination};
use self::paths::register_hipnotic_paths;
use self::scourge::scourge_definition;
use self::spikemine::register_dormant_spikemine;
use self::sword::sword_definition;
use self::wrath::wrath_definition;

/// Register the mission-pack monsters (`registerMissionPackMonsters`).
pub fn register_mission_pack_monsters(
    game: &mut Q1EntityServices,
    pack: Q1MissionPack,
    hooks: MissionMonsterHooks,
) -> Result<Q1MissionPackMonsters, Q1Error> {
    let mut runtime = Q1MissionPackMonsters::new(game, pack, hooks)?;
    if pack == Q1MissionPack::Hipnotic {
        let definition = scourge_definition(&runtime);
        runtime.register(game, definition)?;
        let definition = gremlin_definition(&runtime);
        runtime.register(game, definition)?;
        let definition = armagon_definition(game, &runtime);
        runtime.register(game, definition)?;
        runtime.register(game, decoy_definition())?;
        runtime.register(game, hipnotic_army_definition())?;
        let definition = hipnotic_dog_definition(&runtime);
        runtime.register(game, definition)?;
        register_dormant_spikemine(game)?;
        register_hipnotic_paths(game)?;
    } else {
        register_overlord_destination(game)?;
        register_dragon_corners(game)?;
        runtime.register(game, eel_definition())?;
        runtime.register(game, sword_definition())?;
        runtime.register(game, wrath_definition())?;
        runtime.register(game, mummy_definition())?;
        runtime.register(game, lavaman_definition())?;
        runtime.register(game, overlord_definition())?;
        runtime.register(game, morph_definition())?;
        let definition = dragon_definition(&runtime);
        runtime.register(game, definition)?;
    }
    Ok(runtime)
}

#[cfg(test)]
mod tests {
    use super::{register_mission_pack_monsters, MissionMonsterHooks};
    use crate::q1::missionpacks::types::{test_game, Q1MissionPack};

    #[test]
    fn hipnotic_monsters_register() {
        let mut game = test_game();
        let runtime =
            register_mission_pack_monsters(&mut game, Q1MissionPack::Hipnotic, MissionMonsterHooks::default())
                .expect("register");
        for classname in [
            "monster_scourge",
            "monster_gremlin",
            "monster_armagon",
            "monster_decoy",
            "monster_army",
            "monster_dog",
        ] {
            assert!(runtime.definition(classname).is_some(), "missing {classname}");
        }
    }

    #[test]
    fn rogue_monsters_register() {
        let mut game = test_game();
        let runtime = register_mission_pack_monsters(&mut game, Q1MissionPack::Rogue, MissionMonsterHooks::default())
            .expect("register");
        for classname in [
            "monster_eel",
            "monster_sword",
            "monster_wrath",
            "monster_mummy",
            "monster_lava_man",
            "monster_super_wrath",
            "monster_morph",
            "monster_dragon",
        ] {
            assert!(runtime.definition(classname).is_some(), "missing {classname}");
        }
    }
}
