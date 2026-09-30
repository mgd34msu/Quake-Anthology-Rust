//! Q2 capture the flag (`src/content/q2/multiplayer/ctf`).
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

use std::collections::HashMap;

use qa_core::identity::ActorId;

use crate::q2::equipment::ctf_grapple::Q2CtfGrappleEquipment;
use crate::q2::equipment::grapple_services::CtfGrappleState;
use crate::q2::foundation::host::Q2GameServices;
use crate::q2::support::contracts::SharedGrappleControl;

pub mod checkpoint;
pub mod flags;
pub mod grapple;
pub mod index;
pub mod match_;
pub mod native_grapple_hooks;
pub mod presentation;
pub mod techs;
pub mod types;

pub use self::checkpoint::{Q2CtfCheckpoint, capture_q2_ctf, restore_q2_ctf};
pub use self::flags::Q2CtfFlags;
pub use self::grapple::{Q2CtfGrapple, Q2CtfGrappleBinding};
pub use self::index::Q2Ctf;
pub use self::match_::{Q2CtfAdminSettings, Q2CtfMatch, Q2CtfMatchActions};
pub use self::presentation::Q2CtfPresentation;
pub use self::techs::Q2CtfTechs;
pub use self::types::{
    CTF_FLAGS, Q2CtfElection, Q2CtfElectionKind, Q2CtfEvent, Q2CtfFlagState, Q2CtfForceJoin, Q2CtfGhost, Q2CtfHooks, Q2CtfMatchPhase,
    Q2CtfMatchState, Q2CtfMenuAction, Q2CtfPlayerState, Q2CtfPlayingTeam, Q2CtfRules, Q2CtfScoreRow, Q2CtfTeam, Q2CtfTech,
    create_q2_ctf_rules, ctf_carried_flag, ctf_flag, ctf_name, ctf_player, ctf_print, ctf_score, ctf_team_name, other_ctf_team,
    save_ctf_actor,
};

/// Arena runtime state for CTF.
pub struct CtfRuntime {
    /// Session hooks.
    pub hooks: Option<types::Q2CtfHooks>,
    /// Match rules.
    pub rules: types::Q2CtfRules,
    /// Admitted player states.
    pub states: HashMap<ActorId, types::Q2CtfPlayerState>,
    /// Match state.
    pub match_state: types::Q2CtfMatchState,
    /// Native grapple equipment.
    pub equipment: Option<Q2CtfGrappleEquipment>,
    /// Shared grapple control.
    pub shared: Option<Box<dyn SharedGrappleControl>>,
    /// Inactive grapple states by owner.
    pub inactive_grapple: HashMap<ActorId, CtfGrappleState>,
    /// Queued presentation events.
    pub pending_events: Vec<types::Q2CtfEvent>,
}

impl Default for CtfRuntime {
    fn default() -> Self {
        Self {
            hooks: None,
            rules: types::create_q2_ctf_rules(),
            states: HashMap::new(),
            match_state: types::Q2CtfMatchState::new(),
            equipment: None,
            shared: None,
            inactive_grapple: HashMap::new(),
            pending_events: Vec::new(),
        }
    }
}

impl std::fmt::Debug for CtfRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CtfRuntime")
            .field("hooks", &self.hooks)
            .field("rules", &self.rules)
            .field("states", &self.states)
            .field("match_state", &self.match_state)
            .field("equipment", &self.equipment)
            .field("shared", &self.shared.is_some())
            .field("inactive_grapple", &self.inactive_grapple)
            .field("pending_events", &self.pending_events)
            .finish()
    }
}

/// Session CTF hooks.
pub fn ctf_hooks(game: &Q2GameServices) -> types::Q2CtfHooks {
    game.ctf.hooks.expect("Q2 CTF is not registered")
}

/// Queue a CTF event for composition (`emit` default).
pub fn ctf_emit_event(game: &mut Q2GameServices, event: types::Q2CtfEvent) {
    game.ctf.pending_events.push(event);
}
