//! Client-output mapping into source movement commands and modes.
//!
//! Donor provenance: `src/movement/client-outputs.ts`.

use super::types::{
    ModClientMovementMode, ModClientMovementOutputs, MovementDialect, UserCommand,
};

/// Error for stance commands a source cannot express.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StanceError(pub &'static str);

impl std::fmt::Display for StanceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for StanceError {}

/// Apply a crouch-stance request to a source command following each
/// selected source's ordinary command and clearance checks.
pub fn client_stance_command(
    command: UserCommand,
    crouched: Option<bool>,
) -> Result<UserCommand, StanceError> {
    let Some(crouched) = crouched else {
        return Ok(command);
    };
    match command {
        UserCommand::Q2Rerelease(mut inner) => {
            if crouched {
                inner.buttons |= 16;
            } else {
                inner.buttons &= !16;
            }
            Ok(UserCommand::Q2Rerelease(inner))
        }
        UserCommand::Q1Quakeworld(mut inner) => {
            if crouched {
                inner.up_move = -inner.up_move.abs().max(1.0);
                inner.buttons &= !2;
            } else {
                inner.up_move = inner.up_move.max(0.0);
            }
            Ok(UserCommand::Q1Quakeworld(inner))
        }
        UserCommand::Q2Classic(mut inner) => {
            if crouched {
                inner.up_move = -inner.up_move.abs().max(1.0);
            } else {
                inner.up_move = inner.up_move.max(0.0);
            }
            Ok(UserCommand::Q2Classic(inner))
        }
        UserCommand::Q3(mut inner) => {
            if crouched {
                inner.up_move = -inner.up_move.abs().max(1);
            } else {
                inner.up_move = inner.up_move.max(0);
            }
            Ok(UserCommand::Q3(inner))
        }
        UserCommand::Q1Netquake(_) => Err(StanceError(
            "NetQuake has no source crouch command; authored bounds require their own qualified movement boundary",
        )),
    }
}

/// Map a semantic client mode to the source movement-type word. Original
/// enumerations differ between Classic API3 and rerelease API2023.
#[must_use]
pub fn client_movement_type(kind: MovementDialect, mode: ModClientMovementMode) -> i32 {
    match kind {
        MovementDialect::Q1Netquake => match mode {
            ModClientMovementMode::Normal => 3,
            ModClientMovementMode::Noclip => 8,
            ModClientMovementMode::Freeze => 0,
        },
        MovementDialect::Q1Quakeworld => match mode {
            ModClientMovementMode::Noclip => 1,
            _ => 0,
        },
        MovementDialect::Q2Rerelease => match mode {
            ModClientMovementMode::Normal => 0,
            ModClientMovementMode::Noclip => 2,
            ModClientMovementMode::Freeze => 6,
        },
        MovementDialect::Q2Classic | MovementDialect::Q3 => match mode {
            ModClientMovementMode::Normal => 0,
            ModClientMovementMode::Noclip => 1,
            ModClientMovementMode::Freeze => 4,
        },
    }
}

/// Resolve the effective client movement mode; dead actors report none.
#[must_use]
pub fn client_movement_mode(
    outputs: Option<&ModClientMovementOutputs>,
    health: f64,
) -> Option<ModClientMovementMode> {
    if health > 0.0 {
        outputs.and_then(|outputs| outputs.mode)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::vec3;

    use super::super::types::{
        Q1UserCommand, Q2RereleaseUserCommand, Q2UserCommand, Q3UserCommand, QwUserCommand,
    };

    fn qw_command(up_move: f64, buttons: i32) -> UserCommand {
        UserCommand::Q1Quakeworld(QwUserCommand {
            milliseconds: 20,
            angles: vec3(0.0, 0.0, 0.0),
            forward_move: 0.0,
            side_move: 0.0,
            up_move,
            buttons,
            impulse: 0,
        })
    }

    #[test]
    fn stance_is_identity_without_request() {
        let command = qw_command(40.0, 3);
        assert_eq!(client_stance_command(command, None).unwrap(), command);
    }

    #[test]
    fn quakeworld_stance_drives_up_move_and_clears_jump() {
        let crouched = client_stance_command(qw_command(40.0, 3), Some(true)).unwrap();
        let UserCommand::Q1Quakeworld(inner) = crouched else {
            panic!("dialect changed");
        };
        assert_eq!((inner.up_move, inner.buttons), (-40.0, 1));
        let stood = client_stance_command(qw_command(-40.0, 1), Some(false)).unwrap();
        let UserCommand::Q1Quakeworld(inner) = stood else {
            panic!("dialect changed");
        };
        assert_eq!(inner.up_move, 0.0);
        let forced = client_stance_command(qw_command(0.0, 0), Some(true)).unwrap();
        let UserCommand::Q1Quakeworld(inner) = forced else {
            panic!("dialect changed");
        };
        assert_eq!(inner.up_move, -1.0);
    }

    #[test]
    fn rerelease_stance_toggles_crouch_bit() {
        let command = UserCommand::Q2Rerelease(Q2RereleaseUserCommand {
            milliseconds: 16,
            angles: vec3(0.0, 0.0, 0.0),
            forward_move: 0.0,
            side_move: 0.0,
            buttons: 0,
            server_frame: 1,
        });
        let crouched = client_stance_command(command, Some(true)).unwrap();
        let UserCommand::Q2Rerelease(inner) = crouched else {
            panic!("dialect changed");
        };
        assert_eq!(inner.buttons, 16);
        let stood = client_stance_command(crouched, Some(false)).unwrap();
        let UserCommand::Q2Rerelease(inner) = stood else {
            panic!("dialect changed");
        };
        assert_eq!(inner.buttons, 0);
    }

    #[test]
    fn netquake_stance_is_an_error() {
        let command = UserCommand::Q1Netquake(Q1UserCommand {
            acknowledged_server_time_seconds: 0.0,
            view_angles: vec3(0.0, 0.0, 0.0),
            forward_move: 0.0,
            side_move: 0.0,
            up_move: 0.0,
            buttons: 0,
            impulse: 0,
        });
        assert!(client_stance_command(command, Some(true)).is_err());
        assert!(client_stance_command(command, None).is_ok());
    }

    #[test]
    fn q3_stance_clamps_signed_up_move() {
        let command = UserCommand::Q3(Q3UserCommand {
            server_time_milliseconds: 10,
            angle_words: [0, 0, 0],
            buttons: 0,
            weapon: 1,
            forward_move: 0,
            right_move: 0,
            up_move: 20,
        });
        let crouched = client_stance_command(command, Some(true)).unwrap();
        let UserCommand::Q3(inner) = crouched else {
            panic!("dialect changed");
        };
        assert_eq!(inner.up_move, -20);
        let classic = UserCommand::Q2Classic(Q2UserCommand {
            milliseconds: 16,
            angle_shorts: [0, 0, 0],
            forward_move: 0.0,
            side_move: 0.0,
            up_move: 0.0,
            buttons: 0,
            impulse: 0,
            light_level: 0,
        });
        let crouched = client_stance_command(classic, Some(true)).unwrap();
        let UserCommand::Q2Classic(inner) = crouched else {
            panic!("dialect changed");
        };
        assert_eq!(inner.up_move, -1.0);
    }

    #[test]
    fn movement_type_words_match_source_enumerations() {
        use ModClientMovementMode::{Freeze, Noclip, Normal};
        assert_eq!(client_movement_type(MovementDialect::Q1Netquake, Normal), 3);
        assert_eq!(client_movement_type(MovementDialect::Q1Netquake, Noclip), 8);
        assert_eq!(client_movement_type(MovementDialect::Q1Netquake, Freeze), 0);
        assert_eq!(client_movement_type(MovementDialect::Q1Quakeworld, Noclip), 1);
        assert_eq!(client_movement_type(MovementDialect::Q1Quakeworld, Freeze), 0);
        assert_eq!(client_movement_type(MovementDialect::Q2Rerelease, Normal), 0);
        assert_eq!(client_movement_type(MovementDialect::Q2Rerelease, Noclip), 2);
        assert_eq!(client_movement_type(MovementDialect::Q2Rerelease, Freeze), 6);
        assert_eq!(client_movement_type(MovementDialect::Q2Classic, Freeze), 4);
        assert_eq!(client_movement_type(MovementDialect::Q3, Noclip), 1);
    }

    #[test]
    fn dead_actors_report_no_mode() {
        let outputs = ModClientMovementOutputs {
            mode: Some(ModClientMovementMode::Noclip),
            ..Default::default()
        };
        assert_eq!(client_movement_mode(Some(&outputs), 100.0), Some(ModClientMovementMode::Noclip));
        assert_eq!(client_movement_mode(Some(&outputs), 0.0), None);
        assert_eq!(client_movement_mode(None, 100.0), None);
    }
}
