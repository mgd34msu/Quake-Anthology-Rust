//! Q2 mission-pack monsters (`src/content/q2/missionpacks/monsters`).
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

pub mod boss5;
pub mod chick_heat;
pub mod combat;
pub mod dabeam;
pub mod gladb;
pub mod power_armor;
pub mod rogue_arsenal;
pub mod rogue_common;
pub mod state;
pub mod tables;
pub mod types;

use std::collections::HashMap;
use std::rc::Rc;

use qa_core::identity::ActorId;

use self::state::{RogueFlyerNext, RogueMonsterState};
use self::types::{Q2MissionPackMonsterServices, Q2MissionPackMonsterWeapons};

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
        }
    }
}
