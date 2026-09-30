//! Quake III base/shared: jump pad.
//!
//! Donor provenance: `src/content/q3/base/shared/jump-pad.ts`.

use qa_core::math::{angle_normalize180, vec3, vector_to_angles};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::shared::definitions::*;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jump_pad_applies_velocity_and_event() {
        let mut ps = create_player_state(Product::Baseq3, None);
        ps.pmove_framecount = 9;
        let mut pad = EntityState::new();
        pad.number = 12;
        pad.origin2 = vec3(0.0, 0.0, 700.0);
        touch_jump_pad(&mut ps, &pad);
        assert_eq!(ps.velocity(), vec3(0.0, 0.0, 700.0));
        assert_eq!(ps.jumppad_ent, 12);
        assert_eq!(ps.jumppad_frame, 9);
        assert_eq!(ps.events.get(0), EntityEvent::EvJumpPad as i32);
        assert_eq!(ps.event_parms.get(0), 1);
    }

    #[test]
    fn jump_pad_ignores_flight_and_dead() {
        let mut ps = create_player_state(Product::Baseq3, None);
        ps.powerups.set(Powerup::PwFlight as usize, 9999);
        let mut pad = EntityState::new();
        pad.origin2 = vec3(0.0, 0.0, 700.0);
        touch_jump_pad(&mut ps, &pad);
        assert_eq!(ps.velocity(), vec3(0.0, 0.0, 0.0));
        let mut dead = create_player_state(Product::Baseq3, None);
        dead.pm_type = MoveType::PmDead as i32;
        touch_jump_pad(&mut dead, &pad);
        assert_eq!(dead.jumppad_ent, 0);
    }
}
