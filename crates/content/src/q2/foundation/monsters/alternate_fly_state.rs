//! Alternate fly state (`src/content/q2/foundation/monsters/alternate-fly-state.ts`).
//!
//! Quake II rerelease `monsterinfo_t` fly_* fields (GPL-2.0-or-later).
//!
//! Values are source-private save state. The fields live flattened on
//! [`MonsterState`](super::types::MonsterState); the constructor below
//! seeds them.

use super::types::MonsterState;

/// Seed alternate-fly state (`createAlternateFlyState`).
pub fn seed_alternate_fly_state(state: &mut MonsterState) {
    state.alternate_fly = false;
    state.fly_min_distance = 0.0;
    state.fly_max_distance = 0.0;
    state.fly_acceleration = 0.0;
    state.fly_speed = 0.0;
    state.fly_ideal_position = qa_core::math::Vec3 { x: 0.0, y: 0.0, z: 0.0 };
    state.fly_position_time = 0.0;
    state.fly_buzzard = false;
    state.fly_above = false;
    state.fly_pinned = false;
    state.fly_thrusters = false;
    state.fly_recovery_time = 0.0;
    state.fly_recovery_direction = qa_core::math::Vec3 { x: 0.0, y: 0.0, z: 0.0 };
    state.hint_path = false;
    state.pathing = None;
}
