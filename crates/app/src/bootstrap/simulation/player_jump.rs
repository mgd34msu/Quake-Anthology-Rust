//! Source jump acknowledgements and Q3 character movement-event filtering.
//!
//! Port of donor `src/app/bootstrap/simulation/player-jump.ts`
//! (`movementJumped`, `publishQ3CharacterMovementEvent`).
//!
//! `qa_world` keeps one result type per movement family instead of the donor
//! `MovementResult` union, so callers project their step result into
//! [`MovementJumpResult`] (and the pre-step QuakeWorld buttons, when the
//! previous state is QuakeWorld). Projection is pure data access; every
//! acknowledgement rule below matches the donor branch for branch.

use qa_content::contract::GameFamily;
use qa_content::q3::base::shared::definitions::EntityEvent;
use qa_world::movement::q2::types::pm_flags;
use qa_world::movement::types::{MovementEffect, TraceHit, UserCommand};

/// Jump evidence projected from one family's step result. `None` at the call
/// site means the step did not end active.
#[derive(Debug, Clone, PartialEq)]
pub enum MovementJumpResult {
    /// NetQuake invokes its source `playerAction` hook instead.
    Q1Netquake,
    /// QuakeWorld pre/post-step button and motion evidence.
    Q1Quakeworld {
        /// Post-step button word.
        old_buttons: i32,
        /// Post-step vertical velocity.
        velocity_z: f32,
        /// Post-step dead flag.
        dead: bool,
    },
    /// Quake II classic ground and water evidence.
    Q2Classic {
        /// Post-step ground hit.
        ground: TraceHit,
        /// Post-step water level.
        water_level: i32,
    },
    /// Quake II rerelease jump-sound and flag evidence.
    Q2Rerelease {
        /// Source jump-sound flag.
        jump_sound: bool,
        /// Post-step pmove flags.
        flags: i32,
    },
    /// Quake III step effects.
    Q3 {
        /// Ordered step effects.
        effects: Vec<MovementEffect>,
    },
}

/// Source jump acknowledgements, not a held jump button or an arbitrary upward impulse.
pub fn movement_jumped(
    before_qw_old_buttons: Option<i32>,
    ground: &TraceHit,
    command: &UserCommand,
    result: Option<MovementJumpResult>,
) -> bool {
    let Some(result) = result else {
        return false;
    };
    match result {
        // NetQuake invokes its source playerAction hook.
        MovementJumpResult::Q1Netquake => false,
        MovementJumpResult::Q1Quakeworld {
            old_buttons,
            velocity_z,
            dead,
        } => {
            matches!(before_qw_old_buttons, Some(before) if before & 2 == 0)
                && !matches!(ground, TraceHit::None)
                && old_buttons & 2 != 0
                && velocity_z > 0.0
                && !dead
        }
        MovementJumpResult::Q2Classic {
            ground: after,
            water_level,
        } => {
            let UserCommand::Q2Classic(command) = command else {
                return false;
            };
            !matches!(ground, TraceHit::None)
                && matches!(after, TraceHit::None)
                && command.up_move >= 10.0
                && water_level == 0
        }
        MovementJumpResult::Q2Rerelease { jump_sound, flags } => jump_sound && flags & pm_flags::ON_LADDER == 0,
        MovementJumpResult::Q3 { effects } => effects
            .iter()
            .any(|effect| matches!(effect, MovementEffect::Event(event) if event.event == EntityEvent::EvJump as i32)),
    }
}

/// Native Q3 cgame owns jump voice only for a Q3 character; foreign characters use their shared source cue.
pub fn publish_q3_character_movement_event(character: GameFamily, event: i32) -> bool {
    event != EntityEvent::EvJump as i32 || character == GameFamily::Q3
}

#[cfg(test)]
mod tests {
    use qa_core::identity::ProviderId;
    use qa_world::movement::types::{PredictableMovementEvent, Q2UserCommand};

    use super::*;

    fn qw_command() -> UserCommand {
        UserCommand::Q2Classic(Q2UserCommand {
            milliseconds: 8,
            angle_shorts: [0; 3],
            forward_move: 0.0,
            side_move: 0.0,
            up_move: 0.0,
            buttons: 0,
            impulse: 0,
            light_level: 0,
        })
    }

    fn q2_command(up_move: f64) -> UserCommand {
        let UserCommand::Q2Classic(mut inner) = qw_command() else {
            unreachable!()
        };
        inner.up_move = up_move;
        UserCommand::Q2Classic(inner)
    }

    fn jump_event(number: i32) -> MovementEffect {
        MovementEffect::Event(PredictableMovementEvent {
            provider: ProviderId::new("q3", "movement"),
            sequence: 1,
            event: number,
            parameter: 0,
        })
    }

    #[test]
    fn inactive_or_netquake_never_jumps() {
        assert!(!movement_jumped(Some(0), &TraceHit::None, &qw_command(), None));
        assert!(!movement_jumped(
            Some(0),
            &TraceHit::None,
            &qw_command(),
            Some(MovementJumpResult::Q1Netquake),
        ));
    }

    #[test]
    fn quakeworld_requires_button_edge_and_rise() {
        let risen = MovementJumpResult::Q1Quakeworld {
            old_buttons: 2,
            velocity_z: 1.0,
            dead: false,
        };
        assert!(movement_jumped(
            Some(0),
            &TraceHit::World { model: 0 },
            &qw_command(),
            Some(risen.clone()),
        ));
        assert!(!movement_jumped(
            Some(2),
            &TraceHit::World { model: 0 },
            &qw_command(),
            Some(risen.clone()),
        ));
        assert!(!movement_jumped(
            None,
            &TraceHit::World { model: 0 },
            &qw_command(),
            Some(risen.clone()),
        ));
        assert!(!movement_jumped(
            Some(0),
            &TraceHit::None,
            &qw_command(),
            Some(risen.clone()),
        ));
        let fallen = MovementJumpResult::Q1Quakeworld {
            old_buttons: 2,
            velocity_z: -1.0,
            dead: false,
        };
        assert!(!movement_jumped(
            Some(0),
            &TraceHit::World { model: 0 },
            &qw_command(),
            Some(fallen),
        ));
        let dead = MovementJumpResult::Q1Quakeworld {
            old_buttons: 2,
            velocity_z: 1.0,
            dead: true,
        };
        assert!(!movement_jumped(
            Some(0),
            &TraceHit::World { model: 0 },
            &qw_command(),
            Some(dead),
        ));
    }

    #[test]
    fn q2_classic_requires_launch_and_dry() {
        let air = MovementJumpResult::Q2Classic {
            ground: TraceHit::None,
            water_level: 0,
        };
        assert!(movement_jumped(
            None,
            &TraceHit::World { model: 0 },
            &q2_command(20.0),
            Some(air.clone()),
        ));
        assert!(!movement_jumped(
            None,
            &TraceHit::World { model: 0 },
            &q2_command(9.0),
            Some(air.clone()),
        ));
        let wet = MovementJumpResult::Q2Classic {
            ground: TraceHit::None,
            water_level: 2,
        };
        assert!(!movement_jumped(
            None,
            &TraceHit::World { model: 0 },
            &q2_command(20.0),
            Some(wet),
        ));
        let grounded = MovementJumpResult::Q2Classic {
            ground: TraceHit::World { model: 0 },
            water_level: 0,
        };
        assert!(!movement_jumped(
            None,
            &TraceHit::World { model: 0 },
            &q2_command(20.0),
            Some(grounded),
        ));
    }

    #[test]
    fn q2_rerelease_honors_ladder() {
        let jump = MovementJumpResult::Q2Rerelease {
            jump_sound: true,
            flags: 0,
        };
        assert!(movement_jumped(None, &TraceHit::None, &qw_command(), Some(jump)));
        let ladder = MovementJumpResult::Q2Rerelease {
            jump_sound: true,
            flags: pm_flags::ON_LADDER,
        };
        assert!(!movement_jumped(None, &TraceHit::None, &qw_command(), Some(ladder)));
        let silent = MovementJumpResult::Q2Rerelease {
            jump_sound: false,
            flags: 0,
        };
        assert!(!movement_jumped(None, &TraceHit::None, &qw_command(), Some(silent)));
    }

    #[test]
    fn q3_matches_jump_events_only() {
        let jump = MovementJumpResult::Q3 {
            effects: vec![jump_event(EntityEvent::EvJump as i32)],
        };
        assert!(movement_jumped(None, &TraceHit::None, &qw_command(), Some(jump)));
        let other = MovementJumpResult::Q3 {
            effects: vec![jump_event(EntityEvent::EvFootstep as i32)],
        };
        assert!(!movement_jumped(None, &TraceHit::None, &qw_command(), Some(other)));
        let empty = MovementJumpResult::Q3 { effects: Vec::new() };
        assert!(!movement_jumped(None, &TraceHit::None, &qw_command(), Some(empty)));
    }

    #[test]
    fn q3_jump_voice_stays_native() {
        let jump = EntityEvent::EvJump as i32;
        assert!(publish_q3_character_movement_event(GameFamily::Q3, jump));
        assert!(!publish_q3_character_movement_event(GameFamily::Q1, jump));
        assert!(!publish_q3_character_movement_event(GameFamily::Q2, jump));
        assert!(publish_q3_character_movement_event(
            GameFamily::Q1,
            EntityEvent::EvFootstep as i32
        ));
    }
}
