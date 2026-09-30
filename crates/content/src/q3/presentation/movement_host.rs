//! Quake III presentation: movement host.
//!
//! Donor provenance: `src/content/q3/presentation/movement-host.ts`.

use qa_core::math::{Bounds, Vec3};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::presentation::mirrors_present_scene::*;

// ---------------------------------------------------------------------------
// movement-host.ts
// ---------------------------------------------------------------------------

/// Movement trace with the hit entity (`MovementTrace`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MovementTrace {
    /// Base trace.
    pub base: TraceResult,
    /// Hit entity number.
    pub entity_num: i32,
}

/// Movement callback bundle (`PresentationMovementOptions`).
pub trait PresentationMovementOptions {
    /// Original server time.
    fn original_server_time(&self) -> Option<i32> {
        None
    }
    /// Trace.
    fn trace(&self, start: Vec3, end: Vec3, bounds: Bounds, skip_number: i32, mask: i32) -> MovementTrace;
    /// Point contents.
    fn point_contents(&self, point: Vec3, pass_entity: i32) -> i32;
    /// Trace mask.
    fn trace_mask(&self) -> i32;
    /// Fixed msec, when fixed-step physics is on.
    fn fixed_msec(&self) -> Option<i32>;
    /// Footsteps disabled.
    fn no_footsteps(&self) -> bool;
    /// Gauntlet hit pending.
    fn gauntlet_hit(&self) -> bool;
}

/// Command timing owner (`"q3" | "provider"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandTiming {
    /// Quake III timing.
    Q3,
    /// Provider timing.
    Provider,
}

/// Bounds returned by player movement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoveBounds {
    /// Player bounds.
    pub bounds: Bounds,
}

/// Movement host (`PresentationMovementHost`).
pub trait PresentationMovementHost {
    /// Command timing owner.
    fn command_timing(&self) -> CommandTiming;
    /// Move the player.
    fn move_player(
        &mut self,
        state: &mut SourcePlayerState,
        command: &UserCommand,
        options: &dyn PresentationMovementOptions,
    ) -> MoveBounds;
    /// Update view angles from the command.
    fn update_view_angles(&mut self, state: &mut SourcePlayerState, command: &UserCommand);
}
