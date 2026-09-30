//! Q2 movers (`src/content/q2/foundation/movers.ts`).
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

use std::collections::HashMap;

use qa_core::identity::ActorId;

use super::angular_motion::AngularMoveState;
use super::motion::LinearMoveState;

/// Arena runtime state for this module.
#[derive(Debug, Default)]
pub struct MoverRuntime {
    /// Active angular moves by actor.
    pub angular_moves: HashMap<ActorId, AngularMoveState>,
    /// Active linear moves by actor.
    pub linear_moves: HashMap<ActorId, LinearMoveState>,
}
