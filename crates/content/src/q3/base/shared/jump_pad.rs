//! Quake III base/shared: jump pad.
//!
//! Donor provenance: `src/content/q3/base/shared/jump-pad.ts`.

use qa_core::math::{angle_normalize180, vec3, vector_to_angles};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::shared::definitions_mirror::*;
use crate::q3::base::shared::entity_state::*;
use crate::q3::base::shared::player_state::*;

// ---------------------------------------------------------------------------
// shared/jump-pad.ts
// ---------------------------------------------------------------------------

/// Apply jump-pad velocity to a player state (`BG_TouchJumpPad`).
pub fn touch_jump_pad(state: &mut SourcePlayerState, jump_pad: &EntityState) {
    if state.pm_type != MoveType::PmNormal as i32 || state.powerups.get(Powerup::PwFlight as usize) != 0 {
        return;
    }
    if state.jumppad_ent != jump_pad.number {
        let pitch = angle_normalize180(f64::from(vector_to_angles(jump_pad.origin2).x)).abs();
        state.add_event(EntityEvent::EvJumpPad as i32, i32::from(pitch >= 45.0));
    }
    state.jumppad_ent = jump_pad.number;
    state.jumppad_frame = state.pmove_framecount;
    state.set_velocity(vec3(jump_pad.origin2.x, jump_pad.origin2.y, jump_pad.origin2.z));
}
