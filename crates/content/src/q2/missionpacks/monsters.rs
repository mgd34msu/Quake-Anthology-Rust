//! Q2 mission-pack monsters (`src/content/q2/missionpacks/monsters`).
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

pub mod boss5;
pub mod chick_heat;
pub mod combat;
pub mod dabeam;
pub mod fixbot;
pub mod gekk;
pub mod gladb;
pub mod hints;
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
pub mod xatrix_variants;
pub mod state;
pub mod tables;
pub mod types;

use std::collections::HashMap;
use std::rc::Rc;

use qa_core::identity::ActorId;

use self::state::{MissionPackMonstersCheckpoint, RogueFlyerNext, RogueMonsterState};
use self::types::{Q2MissionPackMonsterServices, Q2MissionPackMonsterWeapons};
use crate::q2::foundation::host::Q2GameServices;
use crate::q2::foundation::monsters::set_hint_paths;

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
pub fn capture_mission_pack_monsters(
    game: &mut Q2GameServices,
    rogue: bool,
) -> MissionPackMonstersCheckpoint {
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
