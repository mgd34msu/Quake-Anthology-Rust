//! Translate physical commands into the source QC ABI.
//! Donor: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/quakec-client-adapter.ts`.

use qa_net::common::commands::UserCommand;

fn netquake(
    view_angles: [f64; 3],
    forward_move: f64,
    side_move: f64,
    up_move: f64,
    buttons: i32,
    impulse: f64,
) -> UserCommand {
    UserCommand::Q1Netquake {
        acknowledged_server_time_seconds: 0.0,
        view_angles,
        forward_move,
        side_move,
        up_move,
        buttons: f64::from(buttons),
        impulse,
    }
}

fn angle_words_to_degrees(words: &[f64; 3]) -> [f64; 3] {
    [
        words[0] * 360.0 / 65536.0,
        words[1] * 360.0 / 65536.0,
        words[2] * 360.0 / 65536.0,
    ]
}

/// Translate physical actions into the source QC ABI, retaining impulses.
#[must_use]
pub fn quake_c_client_command(command: &UserCommand) -> UserCommand {
    match command {
        UserCommand::Q1Netquake { .. } => command.clone(),
        UserCommand::Q1Quakeworld {
            angles,
            forward_move,
            side_move,
            up_move,
            buttons,
            impulse,
            ..
        } => {
            let buttons = *buttons as i32;
            netquake(
                *angles,
                *forward_move,
                *side_move,
                *up_move,
                buttons & 1 | (buttons & 2),
                *impulse,
            )
        }
        UserCommand::Q2Rerelease {
            angles,
            forward_move,
            side_move,
            buttons,
            ..
        } => {
            let jump = (*buttons as i32 & 8) != 0;
            netquake(
                *angles,
                *forward_move,
                *side_move,
                f64::from(i32::from(jump)),
                (*buttons as i32 & 1) | (i32::from(jump) * 2),
                0.0,
            )
        }
        UserCommand::Q2Classic {
            angle_shorts,
            forward_move,
            side_move,
            up_move,
            buttons,
            impulse,
            ..
        } => {
            let jump = *up_move >= 10.0;
            netquake(
                angle_words_to_degrees(angle_shorts),
                *forward_move,
                *side_move,
                *up_move,
                (*buttons as i32 & 1) | (i32::from(jump) * 2),
                *impulse,
            )
        }
        UserCommand::Q3 {
            angle_words,
            buttons,
            forward_move,
            right_move,
            up_move,
            ..
        } => {
            let jump = *up_move >= 10.0;
            netquake(
                angle_words_to_degrees(angle_words),
                *forward_move,
                *right_move,
                *up_move,
                (*buttons as i32 & 1) | (i32::from(jump) * 2),
                0.0,
            )
        }
    }
}

/// Minimal transition state for jump detection. Absorbed; see absorbed-contracts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuakeCTransitionState {
    /// Entity flags.
    pub flags: i32,
    /// Velocity.
    pub velocity: [f64; 3],
}

fn jump_pressed(command: &UserCommand) -> bool {
    match command {
        UserCommand::Q1Netquake { buttons, .. } | UserCommand::Q1Quakeworld { buttons, .. } => {
            (*buttons as i32 & 2) != 0
        }
        UserCommand::Q2Rerelease { buttons, .. } => (*buttons as i32 & 8) != 0,
        UserCommand::Q2Classic { up_move, .. } | UserCommand::Q3 { up_move, .. } => *up_move >= 10.0,
    }
}

/// Whether the command starts a source jump between two states.
#[must_use]
pub fn quake_c_source_jump(
    command: &UserCommand,
    before: &QuakeCTransitionState,
    after: &QuakeCTransitionState,
) -> bool {
    let jump_flags = 512 | 4096;
    jump_pressed(command)
        && (before.flags & jump_flags) == jump_flags
        && (after.flags & jump_flags) == 0
        && after.velocity[2] > before.velocity[2]
}

/// Client movement with its source-jump flag.
#[derive(Debug, Clone, PartialEq)]
pub struct QuakeCClientMovement {
    /// Original command.
    pub command: UserCommand,
    /// Whether it starts a source jump.
    pub source_jump: bool,
}

/// Clear the jump input from a command after the source consumes it.
#[must_use]
pub fn consume_quake_c_jump(command: &UserCommand) -> UserCommand {
    let mut out = command.clone();
    match &mut out {
        UserCommand::Q1Netquake { buttons, .. } | UserCommand::Q1Quakeworld { buttons, .. } => {
            *buttons = f64::from(*buttons as i32 & !2);
        }
        UserCommand::Q2Rerelease { buttons, .. } => {
            *buttons = f64::from(*buttons as i32 & !8);
        }
        UserCommand::Q2Classic { up_move, .. } | UserCommand::Q3 { up_move, .. } => {
            if *up_move > 0.0 {
                *up_move = 0.0;
            }
        }
    }
    out
}

/// Movement state projection for free flight. Absorbed; see absorbed-contracts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MovementState {
    /// NetQuake.
    Q1Netquake {
        /// Move type.
        move_type: i32,
    },
    /// QuakeWorld.
    Q1Quakeworld {
        /// Spectator flag.
        spectator: i32,
    },
    /// Q2 classic.
    Q2Classic {
        /// Move type.
        move_type: i32,
    },
    /// Q2 rerelease.
    Q2Rerelease {
        /// Move type.
        move_type: i32,
    },
    /// Q3.
    Q3 {
        /// Movement type.
        movement_type: i32,
    },
}

/// QW spectators own free flight independently of the physics dialect.
#[must_use]
pub fn quake_c_free_movement(state: &MovementState) -> MovementState {
    match state {
        MovementState::Q1Netquake { .. } => MovementState::Q1Netquake { move_type: 8 },
        MovementState::Q1Quakeworld { .. } => MovementState::Q1Quakeworld { spectator: 1 },
        MovementState::Q2Classic { .. } => MovementState::Q2Classic { move_type: 1 },
        MovementState::Q2Rerelease { .. } => MovementState::Q2Rerelease { move_type: 2 },
        MovementState::Q3 { .. } => MovementState::Q3 { movement_type: 1 },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quakeworld(buttons: f64) -> UserCommand {
        UserCommand::Q1Quakeworld {
            milliseconds: 16.0,
            angles: [0.0, 90.0, 0.0],
            forward_move: 10.0,
            side_move: 4.0,
            up_move: 0.0,
            buttons,
            impulse: 3.0,
        }
    }

    #[test]
    fn netquake_passes_through() {
        let command = netquake([1.0, 2.0, 3.0], 5.0, 6.0, 7.0, 3, 9.0);
        assert_eq!(quake_c_client_command(&command), command);
    }

    #[test]
    fn quakeworld_keeps_impulse_and_jump_bit() {
        match quake_c_client_command(&quakeworld(3.0)) {
            UserCommand::Q1Netquake {
                buttons,
                impulse,
                side_move,
                ..
            } => {
                assert_eq!(buttons, 3.0);
                assert_eq!(impulse, 3.0);
                assert_eq!(side_move, 4.0);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn rerelease_jump_maps_to_up_and_button() {
        let command = UserCommand::Q2Rerelease {
            milliseconds: 16.0,
            angles: [0.0, 0.0, 0.0],
            forward_move: 0.0,
            side_move: 2.0,
            buttons: 9.0,
            server_frame: 1.0,
        };
        match quake_c_client_command(&command) {
            UserCommand::Q1Netquake {
                buttons,
                up_move,
                impulse,
                ..
            } => {
                assert_eq!(buttons, 3.0);
                assert_eq!(up_move, 1.0);
                assert_eq!(impulse, 0.0);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn q3_angles_convert_from_words() {
        let command = UserCommand::Q3 {
            server_time_milliseconds: 100.0,
            angle_words: [16384.0, 32768.0, 0.0],
            buttons: 0.0,
            weapon: 0.0,
            forward_move: 0.0,
            right_move: 5.0,
            up_move: 0.0,
        };
        match quake_c_client_command(&command) {
            UserCommand::Q1Netquake {
                view_angles, side_move, ..
            } => {
                assert_eq!(view_angles, [90.0, 180.0, 0.0]);
                assert_eq!(side_move, 5.0);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn consume_clears_jump_inputs() {
        match consume_quake_c_jump(&quakeworld(3.0)) {
            UserCommand::Q1Quakeworld { buttons, .. } => assert_eq!(buttons, 1.0),
            other => panic!("unexpected command: {other:?}"),
        }
        let before = QuakeCTransitionState {
            flags: 512 | 4096,
            velocity: [0.0, 0.0, 0.0],
        };
        let after = QuakeCTransitionState {
            flags: 0,
            velocity: [0.0, 0.0, 100.0],
        };
        assert!(quake_c_source_jump(&quakeworld(2.0), &before, &after));
        assert!(!quake_c_source_jump(&quakeworld(0.0), &before, &after));
    }

    #[test]
    fn free_movement_sets_dialect_tags() {
        assert_eq!(
            quake_c_free_movement(&MovementState::Q1Netquake { move_type: 3 }),
            MovementState::Q1Netquake { move_type: 8 }
        );
        assert_eq!(
            quake_c_free_movement(&MovementState::Q3 { movement_type: 0 }),
            MovementState::Q3 { movement_type: 1 }
        );
    }
}
