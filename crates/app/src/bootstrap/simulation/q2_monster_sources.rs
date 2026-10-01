//! Selected Q2 monster module registration.
//!
//! Absolute donor:
//! `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/q2-monster-sources.ts`
//!
//! The donor builds `Q2Monsters`/`Q2Ballistics`/`Q2MoverModule` objects,
//! registers base, pack, and rerelease definitions into them, then lets the
//! session create a game over the modules. The Rust content registers
//! directly into the caller's [`Q2GameServices`] (monster state lives in
//! `game.monsters`, ballistics and mover callbacks in the source registries),
//! so this port performs the same registration sequence on a provided game
//! and returns the selected packs. The donor's behavior, weapon, gravity,
//! powerup, and player-effect closures are content-owned in Rust and need no
//! session parameters; `is_n64` and the healthbar transfer survive as
//! options. The rerelease `expansion` selector is not representable (content
//! registers the full rerelease set), and callback wiring the donor performs
//! after game creation happens inside the content registration calls.

use qa_content::monsters::MonsterProgram;
use qa_content::q2::base::monsters::registry::register_q2_classic_base_monsters;
use qa_content::q2::foundation::host::{Q2Edition, Q2GameServices};
use qa_content::q2::missionpacks::monsters::register_q2_mission_pack_monsters;
use qa_content::q2::missionpacks::monsters::types::Q2MonsterMissionPack;
use qa_content::q2::rerelease::monsters::index::register_q2_rerelease_monsters;
use qa_core::identity::ActorId;

/// Registered mission-pack monsters (donor `Q2MissionPackMonsters` module).
pub struct SelectedQ2MonsterPack {
    /// Selected pack.
    pub pack: Q2MonsterMissionPack,
    /// Registered definition names.
    pub monsters: Vec<String>,
}

/// Selected Q2 monster content (donor `SelectedQ2MonsterModules`).
pub struct SelectedQ2MonsterModules {
    /// Registered packs in selection order.
    pub packs: Vec<SelectedQ2MonsterPack>,
}

/// Selected Q2 monster registration options.
pub struct SelectedQ2MonsterOptions {
    /// Source edition.
    pub edition: Q2Edition,
    /// Source program.
    pub program: MonsterProgram,
    /// N64 monster set.
    pub is_n64: bool,
    /// Boss healthbar transfer between phases.
    pub healthbar_transfer: Option<fn(ActorId, ActorId, &mut Q2GameServices)>,
}

/// Mission packs selected for an edition and program.
#[must_use]
pub fn selected_q2_monster_packs(edition: Q2Edition, program: MonsterProgram) -> Vec<Q2MonsterMissionPack> {
    if edition == Q2Edition::Rerelease {
        return vec![Q2MonsterMissionPack::Xatrix, Q2MonsterMissionPack::Rogue];
    }
    match program {
        MonsterProgram::Xatrix => vec![Q2MonsterMissionPack::Xatrix],
        MonsterProgram::Rogue => vec![Q2MonsterMissionPack::Rogue],
        _ => Vec::new(),
    }
}

/// Register the selected Q2 monster content into a game: classic base, then
/// the selected packs, then the rerelease set for rerelease editions.
pub fn register_selected_q2_monster_modules(
    game: &mut Q2GameServices,
    options: &SelectedQ2MonsterOptions,
) -> SelectedQ2MonsterModules {
    register_q2_classic_base_monsters(game);
    let mut packs = Vec::new();
    for pack in selected_q2_monster_packs(options.edition, options.program) {
        let monsters = register_q2_mission_pack_monsters(game, pack, options.edition);
        packs.push(SelectedQ2MonsterPack { pack, monsters });
    }
    if options.edition == Q2Edition::Rerelease {
        register_q2_rerelease_monsters(game, options.is_n64, options.healthbar_transfer);
    }
    SelectedQ2MonsterModules { packs }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rerelease_selects_both_packs() {
        assert_eq!(
            selected_q2_monster_packs(Q2Edition::Rerelease, MonsterProgram::Baseq2),
            vec![Q2MonsterMissionPack::Xatrix, Q2MonsterMissionPack::Rogue]
        );
    }

    #[test]
    fn classic_selects_program_pack_only() {
        assert_eq!(
            selected_q2_monster_packs(Q2Edition::Classic, MonsterProgram::Xatrix),
            vec![Q2MonsterMissionPack::Xatrix]
        );
        assert_eq!(
            selected_q2_monster_packs(Q2Edition::Classic, MonsterProgram::Rogue),
            vec![Q2MonsterMissionPack::Rogue]
        );
    }

    #[test]
    fn classic_base_and_machinegames_select_no_pack() {
        assert!(selected_q2_monster_packs(Q2Edition::Classic, MonsterProgram::Baseq2).is_empty());
        assert!(selected_q2_monster_packs(Q2Edition::Classic, MonsterProgram::Mg2).is_empty());
    }

    // NOTE: no full-registration test: register_q2_classic_base_monsters
    // currently panics inside content validation on faithful donor boss2
    // fidget data (firstFrame 21 > lastFrame 0, donor
    // src/content/q2/base/monsters/tables/boss2.ts:28). That check is
    // stricter than the donor registry and belongs to the content lane; this
    // port calls the registration in donor order and tests the pack
    // selection it owns.
}
