//! Quake III base/shared: entity state.
//!
//! Donor provenance: `src/content/q3/base/shared/entity-state.ts`.

use qa_core::math::{vec3, Vec3};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::shared::trajectory::*;

// ---------------------------------------------------------------------------
// shared/entity-state.ts (via network/q3/state/entity.ts)
// ---------------------------------------------------------------------------

/// Owned `entityState_t` storage (`EntityState`).
///
/// Source enum storage retains raw integer tags; creation follows
/// memset-zero with stationary trajectories.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityState {
    /// Entity number.
    pub number: i32,
    /// Entity type tag.
    pub e_type: i32,
    /// Entity flags.
    pub e_flags: i32,
    /// Position trajectory.
    pub pos: Trajectory,
    /// Angle trajectory.
    pub apos: Trajectory,
    /// Time.
    pub time: i32,
    /// Secondary time.
    pub time2: i32,
    /// Origin.
    pub origin: Vec3,
    /// Secondary origin.
    pub origin2: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Secondary angles.
    pub angles2: Vec3,
    /// Other entity number.
    pub other_entity_num: i32,
    /// Second other entity number.
    pub other_entity_num2: i32,
    /// Ground entity number.
    pub ground_entity_num: i32,
    /// Constant light.
    pub constant_light: i32,
    /// Loop sound.
    pub loop_sound: i32,
    /// Model index.
    pub modelindex: i32,
    /// Second model index.
    pub modelindex2: i32,
    /// Client number.
    pub client_num: i32,
    /// Frame.
    pub frame: i32,
    /// Solid encoding.
    pub solid: i32,
    /// Event.
    pub event: i32,
    /// Event parameter.
    pub event_parm: i32,
    /// Powerup bits.
    pub powerups: i32,
    /// Weapon tag.
    pub weapon: i32,
    /// Legs animation.
    pub legs_anim: i32,
    /// Torso animation.
    pub torso_anim: i32,
    /// Generic value.
    pub generic1: i32,
}

impl Default for EntityState {
    fn default() -> Self {
        Self::new()
    }
}

impl EntityState {
    /// Zero entity state with stationary trajectories.
    #[must_use]
    pub fn new() -> Self {
        let zero = vec3(0.0, 0.0, 0.0);
        Self {
            number: 0,
            e_type: 0,
            e_flags: 0,
            pos: Trajectory::zero(TrajectoryType::TrStationary),
            apos: Trajectory::zero(TrajectoryType::TrStationary),
            time: 0,
            time2: 0,
            origin: zero,
            origin2: zero,
            angles: zero,
            angles2: zero,
            other_entity_num: 0,
            other_entity_num2: 0,
            ground_entity_num: 0,
            constant_light: 0,
            loop_sound: 0,
            modelindex: 0,
            modelindex2: 0,
            client_num: 0,
            frame: 0,
            solid: 0,
            event: 0,
            event_parm: 0,
            powerups: 0,
            weapon: 0,
            legs_anim: 0,
            torso_anim: 0,
            generic1: 0,
        }
    }

    /// Deep copy.
    #[must_use]
    pub fn copy(&self) -> Self {
        self.clone()
    }

    /// Copy every field from a source state.
    pub fn copy_from_state(&mut self, source: &EntityState) {
        copy_entity_state_fields(self, source);
    }
}

/// Copy every entity state field (`copyEntityStateFields`).
pub fn copy_entity_state_fields(target: &mut EntityState, source: &EntityState) {
    *target = source.clone();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linear_fixture() -> Trajectory {
        Trajectory {
            trajectory_type: TrajectoryType::TrLinear,
            time: 1000,
            duration: 0,
            base: vec3(1.0, 2.0, 3.0),
            delta: vec3(100.0, 0.0, -50.0),
        }
    }

    #[test]
    fn entity_state_copies_every_field() {
        let mut source = EntityState::new();
        source.number = 7;
        source.pos = linear_fixture();
        source.origin2 = vec3(1.0, 2.0, 3.0);
        let copy = source.copy();
        assert_eq!(copy, source);
        let mut target = EntityState::new();
        target.copy_from_state(&source);
        assert_eq!(target, source);
        assert_eq!(EntityState::default(), EntityState::new());
    }
}
