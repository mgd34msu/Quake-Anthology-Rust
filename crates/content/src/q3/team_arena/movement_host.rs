//! Quake III team-arena: movement host.
//!
//! Donor provenance: `src/content/q3/team-arena/movement-host.ts`.

use qa_core::identity::ActorId;
use qa_core::math::Bounds;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::team_arena::mirrors::*;

// ---------------------------------------------------------------------------
// movement-host.ts
// ---------------------------------------------------------------------------

/// PMove policy for one client move (`ClientMovementOptions`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientMovementOptions {
    /// Trace mask.
    pub trace_mask: i32,
    /// Fixed millisecond step.
    pub fixed_msec: Option<i32>,
    /// Suppress footsteps.
    pub no_footsteps: bool,
    /// Gauntlet hit this move.
    pub gauntlet_hit: bool,
    /// Debug level.
    pub debug_level: i32,
}

/// Client move outcome (`ClientMovementResult`).
#[derive(Debug, Clone, PartialEq)]
pub struct ClientMovementResult {
    /// Touched actors.
    pub contacts: Vec<ActorId>,
    /// New bounds.
    pub bounds: Bounds,
    /// Water level.
    pub waterlevel: i32,
    /// Water type.
    pub watertype: i32,
    /// Horizontal speed.
    pub xyspeed: f32,
}

/// Movement provider (`ClientMovementHost`).
pub trait MovementHost {
    /// Move a client record.
    fn move_client(
        &self,
        entity: &EntityRef,
        command: &UserCommand,
        options: &ClientMovementOptions,
    ) -> ClientMovementResult;
}
