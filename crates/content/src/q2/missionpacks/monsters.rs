//! Q2 mission-pack monsters barrel (`src/content/q2/missionpacks/monsters/index.ts`).
//!
//! Builds the Xatrix/Rogue monster definition lists, registers them with
//! the monster authority, and re-exports the per-monster definition
//! constructors. `Q2MissionPackMonsterState` is the [`RogueMonsterState`]
//! arena plus the state capture/restore functions; the donor checkpoint
//! `Q2MissionPackMonstersCheckpoint` is [`MissionPackMonstersCheckpoint`].
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

pub mod boss5;
pub mod carrier;
pub mod chick_heat;
pub mod combat;
pub mod dabeam;
pub mod fixbot;
pub mod gekk;
pub mod gladb;
pub mod hints;
pub mod medic;
pub mod power_armor;
pub mod rogue_arsenal;
pub mod rogue_common;
pub mod rogue_flyer;
pub mod rogue_gunner;
pub mod rogue_hover;
pub mod rogue_infantry;
pub mod rogue_jumpers;
pub mod rogue_soldier;
pub mod rogue_variants;
pub mod soldierh;
pub mod spawn;
pub mod stalker;
pub mod state;
pub mod tables;
pub mod turret;
pub mod types;
pub mod widow;
pub mod widow2;
pub mod widow_common;
pub mod widow_death;
pub mod xatrix_variants;

use std::collections::HashMap;
use std::rc::Rc;

use qa_core::identity::ActorId;

pub use self::boss5::boss5_definition;
pub use self::carrier::create_carrier_definition;
pub use self::chick_heat::create_chick_heat_definition;
use self::combat::{create_rogue_combat_hooks, rogue_target_anger};
pub use self::fixbot::create_fixbot_definition;
pub use self::gekk::create_gekk_definition;
pub use self::gladb::create_gladb_definition;
pub use self::medic::create_rogue_medic_definitions;
pub use self::rogue_flyer::create_rogue_flyer_definitions;
pub use self::rogue_hover::create_rogue_hover_definitions;
pub use self::soldierh::create_soldier_heavy_definitions;
pub use self::stalker::create_stalker_definition;
use self::state::RogueFlyerNext;
pub use self::state::{
    capture_mission_monsters, restore_mission_monsters, rogue_state, MissionPackMonstersCheckpoint, RogueMonsterState,
};
pub use self::turret::create_rogue_turret_definition;
use self::types::mission_services;
pub use self::types::{Q2MissionPackMonsterServices, Q2MissionPackMonsterWeapons, Q2MonsterMissionPack};
use crate::q2::base::monsters::boss_common::with_boss_explosion_callbacks;
use crate::q2::foundation::host::{Q2Edition, Q2GameServices};
use crate::q2::foundation::monsters::types::{Q2MonsterDefinition, SourceCombatMode};
use crate::q2::foundation::monsters::{register_monster, set_hint_paths, set_source_combat_rules};

/// Arena runtime state for this module.
pub struct MissionMonsterRuntime {
    /// Projectile provider.
    pub weapons: Option<Rc<dyn Q2MissionPackMonsterWeapons>>,
    /// Monster services.
    pub services: Option<Rc<dyn Q2MissionPackMonsterServices>>,
    /// Rogue state by actor.
    pub rogue: HashMap<ActorId, RogueMonsterState>,
    /// Rogue follow-up move.
    pub flyer_next_move: RogueFlyerNext,
    /// Widow shots fired.
    pub widow_shots_fired: i32,
    /// Widow damage multiplier.
    pub widow_damage_multiplier: u8,
    /// Rogue hint-path state.
    pub hints: hints::RogueHintsState,
    /// Fixbot pre-move origins (source AI/callback scratch within one frame).
    pub fixbot_before_move: HashMap<ActorId, qa_core::math::Vec3>,
}

impl Default for MissionMonsterRuntime {
    fn default() -> Self {
        Self {
            weapons: None,
            services: None,
            rogue: HashMap::new(),
            flyer_next_move: RogueFlyerNext::default(),
            widow_shots_fired: 0,
            widow_damage_multiplier: 1,
            hints: hints::RogueHintsState::default(),
            fixbot_before_move: HashMap::new(),
        }
    }
}

/// Register Rogue hint paths (`new Q2RogueHints` + `setHintPaths`).
pub fn register_q2_rogue_hint_paths(game: &mut Q2GameServices) {
    let module = hints::hint_path_module();
    game.source_callbacks.register(&module.callbacks);
    game.modules.push(module);
    set_hint_paths(game, Box::new(hints::RogueHints));
}

/// Capture mission-pack monsters (`Q2MissionPackMonsters.capture`).
pub fn capture_mission_pack_monsters(game: &mut Q2GameServices, rogue: bool) -> MissionPackMonstersCheckpoint {
    let mut checkpoint = state::capture_mission_monsters(game);
    checkpoint.hints = if rogue {
        Some(hints::capture_rogue_hints(game))
    } else {
        None
    };
    checkpoint
}

/// Restore mission-pack monsters (`Q2MissionPackMonsters.restore`).
pub fn restore_mission_pack_monsters(
    game: &mut Q2GameServices,
    checkpoint: &MissionPackMonstersCheckpoint,
    rogue: bool,
) {
    state::restore_mission_monsters(game, checkpoint);
    if let Some(saved) = &checkpoint.hints {
        if !rogue {
            panic!("Rogue hint paths restored without their selected source module");
        }
        hints::restore_rogue_hints(game, saved);
    }
}

/// Xatrix monster definitions (`q2XatrixMonsterDefinitions`).
pub fn q2_xatrix_monster_definitions() -> Vec<Q2MonsterDefinition> {
    let mut definitions = vec![
        gekk::create_gekk_definition(),
        fixbot::create_fixbot_definition(),
        gladb::create_gladb_definition(),
        with_boss_explosion_callbacks(boss5::boss5_definition()),
        chick_heat::create_chick_heat_definition(),
    ];
    definitions.extend(soldierh::create_soldier_heavy_definitions());
    definitions.extend(xatrix_variants::create_xatrix_base_variants());
    definitions
}

/// Rogue monster definitions (`q2RogueMonsterDefinitions`).
pub fn q2_rogue_monster_definitions() -> Vec<Q2MonsterDefinition> {
    let mut definitions = vec![
        stalker::create_stalker_definition(),
        turret::create_rogue_turret_definition(),
        carrier::create_carrier_definition(),
        widow::create_widow_definition(),
        widow2::create_widow2_definition(),
        rogue_gunner::create_rogue_gunner_definition(),
    ];
    definitions.extend(rogue_flyer::create_rogue_flyer_definitions());
    definitions.extend(rogue_hover::create_rogue_hover_definitions());
    definitions.extend(medic::create_rogue_medic_definitions());
    definitions.extend(rogue_variants::create_rogue_base_variants());
    definitions.extend(rogue_arsenal::create_rogue_arsenal_monsters());
    definitions.extend(rogue_jumpers::create_rogue_jumping_monsters());
    definitions
}

/// Original mission-pack fallbacks (`q2OriginalMissionPackFallbacks`).
pub const Q2_ORIGINAL_MISSION_PACK_FALLBACKS: [&str; 16] = [
    "monster_gekk",
    "monster_fixbot",
    "monster_gladb",
    "monster_boss5",
    "monster_chick_heat",
    "monster_soldier_ripper",
    "monster_soldier_hypergun",
    "monster_soldier_lasergun",
    "monster_stalker",
    "monster_kamikaze",
    "monster_daedalus",
    "monster_turret",
    "monster_carrier",
    "monster_medic_commander",
    "monster_widow",
    "monster_widow2",
];

/// Register mission-pack monsters (`registerQ2MissionPackMonsters`).
///
/// Returns the fallback classnames (`originalSourceFallbacks`). Spawn
/// dispatch composes structurally: the hint-path module answers
/// `hint_path` spawns and the monster registry answers the rest.
pub fn register_q2_mission_pack_monsters(
    game: &mut Q2GameServices,
    pack: Q2MonsterMissionPack,
    edition: Q2Edition,
) -> Vec<String> {
    let rogue = pack == Q2MonsterMissionPack::Rogue;
    if rogue {
        register_q2_rogue_hint_paths(game);
    }
    game.source_callbacks.register(&dabeam::monster_dabeam_callbacks());
    let definitions = if rogue {
        q2_rogue_monster_definitions()
    } else {
        q2_xatrix_monster_definitions()
    };
    let fallbacks: Vec<String> = definitions
        .iter()
        .filter(|definition| Q2_ORIGINAL_MISSION_PACK_FALLBACKS.contains(&definition.classname.as_str()))
        .map(|definition| definition.classname.clone())
        .collect();
    for definition in definitions {
        if edition == Q2Edition::Classic {
            register_monster(game, definition.clone(), Some(Q2Edition::Classic));
        }
        if Q2_ORIGINAL_MISSION_PACK_FALLBACKS.contains(&definition.classname.as_str()) {
            register_monster(game, definition, None);
        }
    }
    if rogue {
        let services = mission_services(game);
        let hooks = create_rogue_combat_hooks(services);
        set_source_combat_rules(game, SourceCombatMode::Rogue, Some(Box::new(hooks)));
    } else {
        set_source_combat_rules(game, SourceCombatMode::Base, None);
    }
    fallbacks
}

/// Mission-pack target anger (`Q2MissionPackMonsters.targetAnger`).
pub fn mission_pack_target_anger(game: &mut Q2GameServices, entity: &ActorId, target: &ActorId) {
    rogue_target_anger(game, entity, target);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classnames(definitions: &[Q2MonsterDefinition]) -> Vec<&str> {
        definitions
            .iter()
            .map(|definition| definition.classname.as_str())
            .collect()
    }

    #[test]
    fn xatrix_definitions_carry_source_classnames() {
        let definitions = q2_xatrix_monster_definitions();
        let names = classnames(&definitions);
        for expected in [
            "monster_gekk",
            "monster_fixbot",
            "monster_gladb",
            "monster_boss5",
            "monster_chick_heat",
        ] {
            assert!(names.contains(&expected), "missing {expected}");
        }
        assert_eq!(boss5_definition().classname, "monster_boss5");
        assert_eq!(create_gekk_definition().classname, "monster_gekk");
    }

    #[test]
    fn rogue_definitions_carry_source_classnames() {
        let definitions = q2_rogue_monster_definitions();
        let names = classnames(&definitions);
        for expected in [
            "monster_stalker",
            "monster_turret",
            "monster_carrier",
            "monster_widow",
            "monster_widow2",
        ] {
            assert!(names.contains(&expected), "missing {expected}");
        }
        assert_eq!(create_stalker_definition().classname, "monster_stalker");
        assert_eq!(create_carrier_definition().classname, "monster_carrier");
    }

    #[test]
    fn fallbacks_match_source_set() {
        assert_eq!(Q2_ORIGINAL_MISSION_PACK_FALLBACKS.len(), 16);
        let xatrix = q2_xatrix_monster_definitions();
        let rogue = q2_rogue_monster_definitions();
        let mut names = classnames(&xatrix);
        names.extend(classnames(&rogue));
        for fallback in Q2_ORIGINAL_MISSION_PACK_FALLBACKS {
            assert!(names.contains(&fallback), "missing {fallback}");
        }
    }
}
