//! Q2 LMCTF barrel (`src/content/q2/multiplayer/lmctf/index.ts`).
//!
//! Pure re-export barrel: the donor index only re-exports. The donor's
//! `LmctfContext` is the [`LmctfRuntime`] arena reached through
//! `&mut Q2GameServices`, so the context-taking donor functions are the
//! free functions re-exported below.
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

use std::collections::HashMap;

use qa_core::identity::ActorId;

use crate::q2::equipment::grapple_services::LmctfGrappleState;
use crate::q2::equipment::lmctf_grapple::LmctfGrappleEquipment;
use crate::q2::foundation::host::Q2GameServices;
use crate::q2::support::contracts::SharedGrappleControl;

pub mod admin;
pub mod flags;
pub mod grapple;
pub mod match_;
pub mod presentation;
pub mod runes;
pub mod runtime;
pub mod spawns;
pub mod types;
pub mod vote;
pub mod weapons;

pub use self::admin::lmctf_admin_command;
pub use self::flags::{lmctf_flag_callbacks, LmctfFlagSlot, LmctfFlagState, LmctfFlags, LmctfFlagsCheckpoint};
pub use self::grapple::{hook_definition, lmctf_grapple_equipment, LmctfGrapple, LmctfGrappleExtension};
pub use self::match_::{LmctfMatch, LmctfMatchCheckpoint, LmctfMatchPhase, LmctfMatchState};
pub use self::presentation::{lmctf_menu, lmctf_scoreboard};
pub use self::runes::{lmctf_rune_callbacks, LmctfRunes, LmctfRunesCheckpoint};
pub use self::runtime::{
    lmctf_callbacks, lmctf_spawn, LmctfCheckpoint, LmctfPlayerCheckpoint, LmctfRulesCheckpoint, Q2Lmctf,
};
pub use self::spawns::{lmctf_team_spawn, select_lmctf_spawn};
pub use self::types::{
    create_lmctf_rules, lmctf_active, lmctf_name, lmctf_player, lmctf_print, lmctf_score, lmctf_stat, lmctf_toss,
    LmctfEvent, LmctfHooks, LmctfMapChange, LmctfMenuEntry, LmctfPlayerState, LmctfPlayingTeam, LmctfRules, LmctfRune,
    LmctfRuneDefinition, LmctfScoreRow, LmctfTeam, LmctfTravel, LmctfTravelPlayer, LMCTF_RUNES,
};
pub use self::vote::{LmctfVote, LmctfVoteCheckpoint};
pub use self::weapons::{lmctf_plasma, lmctf_weapon_callbacks, LmctfPlasmaExtension, LmctfWeapons, LMCTF_PLASMA_ITEM};

/// Arena runtime state for LMCTF.
pub struct LmctfRuntime {
    /// Session hooks.
    pub hooks: Option<types::LmctfHooks>,
    /// Match rules.
    pub rules: types::LmctfRules,
    /// Admitted player states.
    pub states: HashMap<ActorId, types::LmctfPlayerState>,
    /// Match state.
    pub match_state: match_::LmctfMatchState,
    /// Native grapple equipment.
    pub equipment: Option<LmctfGrappleEquipment>,
    /// Shared grapple control.
    pub shared: Option<Box<dyn SharedGrappleControl>>,
    /// Inactive grapple states by owner.
    pub inactive_grapple: HashMap<ActorId, LmctfGrappleState>,
    /// Flag actors by team.
    pub flag_slots: HashMap<u8, ActorId>,
    /// Last flag-taken sound time.
    pub flag_taken_sound: f64,
    /// Rune animation direction.
    pub runes_forward: bool,
    /// Vote start time.
    pub vote_started: Option<f64>,
    /// Whether the plasma quad is live.
    pub plasma_quad: bool,
    /// Pending time-travel.
    pub travel: Option<types::LmctfTravel>,
}

impl Default for LmctfRuntime {
    fn default() -> Self {
        Self {
            hooks: None,
            rules: types::create_lmctf_rules(),
            states: HashMap::new(),
            match_state: match_::LmctfMatchState::default(),
            equipment: None,
            shared: None,
            inactive_grapple: HashMap::new(),
            flag_slots: HashMap::new(),
            flag_taken_sound: 0.0,
            runes_forward: true,
            vote_started: None,
            plasma_quad: false,
            travel: None,
        }
    }
}

impl std::fmt::Debug for LmctfRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LmctfRuntime")
            .field("hooks", &self.hooks)
            .field("rules", &self.rules)
            .field("states", &self.states)
            .field("match_state", &self.match_state)
            .field("equipment", &self.equipment)
            .field("shared", &self.shared.is_some())
            .field("inactive_grapple", &self.inactive_grapple)
            .field("flag_slots", &self.flag_slots)
            .field("flag_taken_sound", &self.flag_taken_sound)
            .field("runes_forward", &self.runes_forward)
            .field("vote_started", &self.vote_started)
            .field("plasma_quad", &self.plasma_quad)
            .field("travel", &self.travel)
            .finish()
    }
}

/// Session LMCTF hooks.
pub fn lmctf_hooks(game: &Q2GameServices) -> types::LmctfHooks {
    game.lmctf.hooks.expect("Q2 LMCTF is not registered")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runes_cover_five_distinct_bits() {
        assert_eq!(LMCTF_RUNES.len(), 5);
        let mut bits: Vec<i32> = LMCTF_RUNES.iter().map(|rune| rune.bit).collect();
        bits.sort_unstable();
        assert_eq!(bits, vec![1, 2, 4, 8, 16]);
        assert!(LMCTF_RUNES.iter().any(|rune| rune.kind == LmctfRune::Vampire));
    }

    #[test]
    fn default_rules_match_donor() {
        let rules = create_lmctf_rules();
        assert_eq!(rules, LmctfRuntime::default().rules);
        assert_eq!(rules.runes, 15);
        assert_eq!(rules.countdown_seconds, 15.0);
        assert_eq!(rules.quad_seconds, 30.0);
        assert!(rules.map_list.is_empty());
    }

    #[test]
    fn runtime_defaults_to_quiet_match() {
        let runtime = LmctfRuntime::default();
        assert!(runtime.hooks.is_none());
        assert!(runtime.states.is_empty());
        assert!(runtime.travel.is_none());
        assert!(!runtime.plasma_quad);
    }
}
