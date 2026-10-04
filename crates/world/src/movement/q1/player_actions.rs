//! Quake I player jump and water-jump actions.
//!
//! Donor provenance: `src/movement/q1/player-actions.ts` (movement portions
//! of Quake `progs106/client.qc`, callable by the game provider).

use qa_core::math::Vec3;

use super::common::{seconds, MovementContext};
use super::types::{
    Q1MovementHooks, Q1MovementInput, Q1MovementOptions, Q1MovementServices, Q1MovementState, Q1PlayerInput, Q1State,
    Q1TraceMove, Q1_CONTENTS_SLIME, Q1_CONTENTS_WATER, Q1_FLAG_JUMPRELEASED, Q1_FLAG_ONGROUND, Q1_FLAG_WATERJUMP,
};

/// Jump result action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1JumpAction {
    /// No action taken.
    None,
    /// Ground jump.
    Jump,
    /// Swim stroke.
    Swim,
}

/// Jump outcome.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1JumpResult {
    /// Updated state.
    pub state: Q1MovementState,
    /// Action taken.
    pub action: Q1JumpAction,
}

/// Ground-jump vertical impulse shared by NetQuake gamecode and QuakeWorld
/// pmove.
pub const Q1_JUMP_IMPULSE: f64 = 270.0;

/// Swim vertical impulse by water contents. NetQuake gamecode (`client.qc`)
/// and QuakeWorld pmove (`pmove.c`) carry the same table.
#[must_use]
pub fn q1_swim_vertical(water_type: i32) -> f64 {
    if water_type == Q1_CONTENTS_WATER {
        100.0
    } else if water_type == Q1_CONTENTS_SLIME {
        80.0
    } else {
        50.0
    }
}

/// Gamecode jump/swim entry. Gamecode owns sounds and timers; do not run in
/// addition to QC.
#[must_use]
pub fn q1_player_jump<S: Q1MovementServices>(state: &Q1MovementState, services: &S) -> Q1JumpResult {
    if state.flags & Q1_FLAG_WATERJUMP != 0 {
        return Q1JumpResult {
            state: state.clone(),
            action: Q1JumpAction::None,
        };
    }
    if state.water_level >= 2 {
        let vertical = q1_swim_vertical(state.water_type);
        return Q1JumpResult {
            state: Q1MovementState {
                velocity: Vec3 {
                    x: state.velocity.x,
                    y: state.velocity.y,
                    z: vertical as f32,
                },
                ..state.clone()
            },
            action: Q1JumpAction::Swim,
        };
    }
    if state.flags & Q1_FLAG_ONGROUND == 0 || state.flags & Q1_FLAG_JUMPRELEASED == 0 {
        return Q1JumpResult {
            state: state.clone(),
            action: Q1JumpAction::None,
        };
    }
    let numeric = services.numeric();
    Q1JumpResult {
        state: Q1MovementState {
            flags: state.flags & !(Q1_FLAG_ONGROUND | Q1_FLAG_JUMPRELEASED),
            velocity: Vec3 {
                x: state.velocity.x,
                y: state.velocity.y,
                z: numeric.store(numeric.add(f64::from(state.velocity.z), Q1_JUMP_IMPULSE)),
            },
            ..state.clone()
        },
        action: Q1JumpAction::Jump,
    }
}

/// Original QC's point traces and 225-unit impulse differ from QW's 310
/// impulse.
pub fn q1_check_water_jump<S: Q1MovementServices, H: Q1MovementHooks>(
    input: &Q1MovementInput,
    services: &mut S,
    options: Q1MovementOptions<H>,
) -> Q1MovementState {
    let mut context = MovementContext::new(Q1PlayerInput::Netquake(input.clone()), services, options);
    let frame_seconds = seconds(input.fields.frame.time);
    q1_check_water_jump_in(&mut context, &input.state, frame_seconds)
}

/// Water-jump probe against a live step context (NetQuake player actions).
pub fn q1_check_water_jump_in<S: Q1MovementServices, H: Q1MovementHooks>(
    context: &mut MovementContext<'_, S, H>,
    state: &Q1MovementState,
    frame_time_seconds: f64,
) -> Q1MovementState {
    let state = state.clone();
    let axes = context.math.angles(state.angles);
    // progs106 ignores normalize(v_forward)'s return value after zeroing Z.
    let forward = context
        .math
        .vec(f64::from(axes.forward.x), f64::from(axes.forward.y), 0.0);
    let up8 = context.math.vec(0.0, 0.0, 8.0);
    let start = context.math.add(state.origin, up8);
    let end = context.math.ma(start, 24.0, forward);
    let low = context.trace(
        start,
        end,
        super::super::types::TraceShape::Point,
        Q1TraceMove::NoMonsters,
    );
    if low.fraction == 1.0 {
        return state;
    }
    let bounds = context.bounds();
    let n = context.math.n;
    let rise = context.math.vec(0.0, 0.0, n.sub(f64::from(bounds.max.z), 8.0));
    let start = context.math.add(start, rise);
    let direction = context.math.scale(low.source_plane.normal, -50.0);
    let end = context.math.ma(start, 24.0, forward);
    let high = context.trace(
        start,
        end,
        super::super::types::TraceShape::Point,
        Q1TraceMove::NoMonsters,
    );
    if high.fraction != 1.0 {
        return Q1MovementState {
            water_jump_direction: direction,
            ..state
        };
    }
    Q1MovementState {
        flags: (state.flags | Q1_FLAG_WATERJUMP) & !Q1_FLAG_JUMPRELEASED,
        velocity: context
            .math
            .vec(f64::from(state.velocity.x), f64::from(state.velocity.y), 225.0),
        water_jump_direction: direction,
        teleport_time_seconds: f64::from(n.store(n.add(frame_time_seconds, 2.0))),
        ..state
    }
}

/// Q1 union-state view for shared call sites.
#[must_use]
pub fn as_union(state: &Q1MovementState) -> Q1State {
    Q1State::Netquake(state.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;

    use super::super::types::Q1_CONTENTS_EMPTY;

    #[test]
    fn water_jump_blocks_regular_jump() {
        let owner = IdentityOwner::create("q1-actions").unwrap();
        let _ = owner;
        let services = NullActions;
        let state = jump_state(Q1_FLAG_WATERJUMP, 0, Q1_CONTENTS_WATER);
        let result = q1_player_jump(&state, &services);
        assert_eq!(result.action, Q1JumpAction::None);
    }

    #[test]
    fn swim_impulse_matches_water_type() {
        let services = NullActions;
        let water = q1_player_jump(&jump_state(0, 2, Q1_CONTENTS_WATER), &services);
        assert_eq!((water.action, water.state.velocity.z), (Q1JumpAction::Swim, 100.0));
        let slime = q1_player_jump(&jump_state(0, 3, Q1_CONTENTS_SLIME), &services);
        assert_eq!((slime.action, slime.state.velocity.z), (Q1JumpAction::Swim, 80.0));
        let lava = q1_player_jump(&jump_state(0, 2, -5), &services);
        assert_eq!((lava.action, lava.state.velocity.z), (Q1JumpAction::Swim, 50.0));
    }

    #[test]
    fn shared_jump_table_matches_both_donors() {
        assert_eq!(Q1_JUMP_IMPULSE, 270.0);
        assert_eq!(q1_swim_vertical(Q1_CONTENTS_WATER), 100.0);
        assert_eq!(q1_swim_vertical(Q1_CONTENTS_SLIME), 80.0);
        assert_eq!(q1_swim_vertical(-5), 50.0);
        assert_eq!(q1_swim_vertical(Q1_CONTENTS_EMPTY), 50.0);
    }

    #[test]
    fn ground_jump_needs_ground_and_release() {
        let services = NullActions;
        let airborne = q1_player_jump(&jump_state(Q1_FLAG_JUMPRELEASED, 0, Q1_CONTENTS_WATER), &services);
        assert_eq!(airborne.action, Q1JumpAction::None);
        let held = q1_player_jump(&jump_state(Q1_FLAG_ONGROUND, 0, Q1_CONTENTS_WATER), &services);
        assert_eq!(held.action, Q1JumpAction::None);
        let jumping = q1_player_jump(
            &jump_state(Q1_FLAG_ONGROUND | Q1_FLAG_JUMPRELEASED, 0, Q1_CONTENTS_WATER),
            &services,
        );
        assert_eq!(jumping.action, Q1JumpAction::Jump);
        assert_eq!(jumping.state.velocity.z, 270.0);
        assert_eq!(jumping.state.flags & (Q1_FLAG_ONGROUND | Q1_FLAG_JUMPRELEASED), 0);
    }

    fn jump_state(flags: i32, water_level: i32, water_type: i32) -> Q1MovementState {
        Q1MovementState {
            origin: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            angles: vec3(0.0, 0.0, 0.0),
            old_origin: vec3(0.0, 0.0, 0.0),
            angular_velocity: vec3(0.0, 0.0, 0.0),
            view_angles: vec3(0.0, 0.0, 0.0),
            punch_angles: vec3(0.0, 0.0, 0.0),
            move_type: 3,
            flags,
            ground: super::super::super::types::TraceHit::None,
            water_level,
            water_type,
            teleport_time_seconds: 0.0,
            water_jump_direction: vec3(0.0, 0.0, 0.0),
            ideal_pitch: 0.0,
            fix_angle: false,
            health: 100.0,
        }
    }

    struct NullActions;

    impl Q1MovementServices for NullActions {
        fn numeric(&self) -> qa_core::numeric::NumericOps {
            qa_core::numeric::NumericOps::select(qa_core::numeric::Q1_DONOR_PROFILE).unwrap()
        }
        fn trace(&mut self, _query: super::super::types::Q1TraceQuery) -> super::super::types::Q1Trace {
            panic!("no traces in jump tests")
        }
        fn point_contents(&mut self, _point: Vec3) -> i32 {
            -1
        }
        fn touch(
            &mut self,
            _contact: super::super::super::types::MovementTouchContact,
            state: Q1State,
        ) -> super::super::super::types::MovementContinuation<Q1State> {
            super::super::super::types::MovementContinuation::Continue(state)
        }
        fn weapon_step(
            &mut self,
            _input: super::super::types::Q1WeaponStepInput<'_>,
            _state: &Q1State,
        ) -> super::super::types::Q1WeaponStepResult {
            panic!("no weapon in jump tests")
        }
        fn animation_step(
            &mut self,
            _input: super::super::types::Q1AnimationStepInput<'_>,
        ) -> super::super::types::Q1AnimationStepResult {
            panic!("no animation in jump tests")
        }
    }

    #[test]
    fn union_view_preserves_state() {
        let state = jump_state(Q1_FLAG_ONGROUND, 0, Q1_CONTENTS_WATER);
        let union = as_union(&state);
        assert_eq!(union, Q1State::Netquake(state));
    }
}
