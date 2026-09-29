//! Client command application ported from
//! `src/world/session/mod-client-command.ts` and
//! `src/world/session/mod-client-input-values.ts`. Source command shapes
//! differ per family; the selected wire codec never chooses movement. This
//! module pins the donor scales, button bits, and ABI range checks.

use crate::WorldError;

/// Source command family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientFamily {
    /// NetQuake.
    Q1Netquake,
    /// QuakeWorld.
    Q1Quakeworld,
    /// Classic Quake II.
    Q2Classic,
    /// Rerelease Quake II.
    Q2Rerelease,
    /// Quake III.
    Q3,
}

/// Attack button bit, shared by every family.
pub const ATTACK_BUTTON: i32 = 1;
/// Jump button bit for Q1 families.
pub const Q1_JUMP_BUTTON: i32 = 2;
/// Jump button bit for Q2 rerelease.
pub const Q2R_JUMP_BUTTON: i32 = 8;
/// Crouch/down button bit for Q2 rerelease.
pub const Q2R_DOWN_BUTTON: i32 = 16;
/// Up-move threshold that counts as jump for Q2 classic and Q3.
pub const JUMP_UP_MOVE: i32 = 10;
/// Maximum impulse value (one byte).
pub const MAX_IMPULSE: i32 = 255;

/// Movement scale per family (`modClientMoveScale`): Q3 uses 127, Q1
/// families 320, Q2 families 200.
#[must_use]
pub const fn move_scale(family: ClientFamily) -> f64 {
    match family {
        ClientFamily::Q3 => 127.0,
        ClientFamily::Q1Netquake | ClientFamily::Q1Quakeworld => 320.0,
        ClientFamily::Q2Classic | ClientFamily::Q2Rerelease => 200.0,
    }
}

/// Whether this family carries an impulse byte (Q3 and Q2 rerelease do not).
#[must_use]
pub const fn carries_impulse(family: ClientFamily) -> bool {
    match family {
        ClientFamily::Q3 | ClientFamily::Q2Rerelease => false,
        ClientFamily::Q1Netquake | ClientFamily::Q1Quakeworld | ClientFamily::Q2Classic => true,
    }
}

/// Source client command in per-family units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClientCommand {
    /// Command family.
    pub family: ClientFamily,
    /// Button bitmask.
    pub buttons: i32,
    /// Weapon impulse byte (families that carry one).
    pub impulse: i32,
    /// Forward move in family units.
    pub forward_move: f64,
    /// Side move in family units (Q1/Q2).
    pub side_move: f64,
    /// Right move in family units (Q3).
    pub right_move: f64,
    /// Up move in family units.
    pub up_move: f64,
}

impl ClientCommand {
    /// Zero command for a family.
    #[must_use]
    pub const fn zero(family: ClientFamily) -> Self {
        Self {
            family,
            buttons: 0,
            impulse: 0,
            forward_move: 0.0,
            side_move: 0.0,
            right_move: 0.0,
            up_move: 0.0,
        }
    }

    /// Lateral move: `rightMove` for Q3, `sideMove` otherwise.
    #[must_use]
    pub const fn lateral(&self) -> f64 {
        match self.family {
            ClientFamily::Q3 => self.right_move,
            _ => self.side_move,
        }
    }

    /// Jump input (`modClientInputValues`): button bits for Q1 and Q2
    /// rerelease, up-move threshold otherwise.
    #[must_use]
    pub fn jump_pressed(&self) -> bool {
        match self.family {
            ClientFamily::Q1Netquake | ClientFamily::Q1Quakeworld => self.buttons & Q1_JUMP_BUTTON != 0,
            ClientFamily::Q2Rerelease => self.buttons & Q2R_JUMP_BUTTON != 0,
            ClientFamily::Q2Classic | ClientFamily::Q3 => self.up_move >= f64::from(JUMP_UP_MOVE),
        }
    }

    /// Attack input: bit 1 in every family.
    #[must_use]
    pub fn attack_pressed(&self) -> bool {
        self.buttons & ATTACK_BUTTON != 0
    }

    /// Effective impulse: the donor forces 0 for Q3 and Q2 rerelease.
    #[must_use]
    pub fn effective_impulse(&self) -> i32 {
        if carries_impulse(self.family) {
            self.impulse
        } else {
            0
        }
    }
}

/// Normalized client input in `[-1, 1]` movement units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClientInput {
    /// Attack held.
    pub attack: bool,
    /// Jump held.
    pub jump: bool,
    /// Impulse byte.
    pub impulse: i32,
    /// Normalized forward move.
    pub forward: f64,
    /// Normalized lateral move.
    pub lateral: f64,
    /// Normalized up move.
    pub up: f64,
}

impl ClientInput {
    /// Normalize a source command (`modClientInputValues`).
    #[must_use]
    pub fn normalized(command: &ClientCommand) -> Self {
        let scale = move_scale(command.family);
        let up = match command.family {
            ClientFamily::Q2Rerelease => {
                if command.buttons & Q2R_JUMP_BUTTON != 0 {
                    1.0
                } else if command.buttons & Q2R_DOWN_BUTTON != 0 {
                    -1.0
                } else {
                    0.0
                }
            }
            _ => command.up_move / scale,
        };
        Self {
            attack: command.attack_pressed(),
            jump: command.jump_pressed(),
            impulse: command.effective_impulse(),
            forward: command.forward_move / scale,
            lateral: command.lateral() / scale,
            up,
        }
    }
}

/// Scalar client input channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarInput {
    /// Attack button.
    Attack,
    /// Jump button or up-move.
    Jump,
    /// Weapon impulse byte.
    Impulse,
    /// Forward move.
    ForwardMove,
    /// Side/right move.
    SideMove,
    /// Up move.
    UpMove,
}

/// Apply a normalized scalar to a command (`modClientCommandScalar`),
/// enforcing the destination command ABI ranges.
pub fn apply_scalar(command: &ClientCommand, input: ScalarInput, value: f64) -> Result<ClientCommand, WorldError> {
    if !value.is_finite() {
        return Err(WorldError::BadClientCommand(
            "Source input output must be finite".to_string(),
        ));
    }
    let bit = |mask: i32, enabled: bool| {
        if enabled {
            command.buttons | mask
        } else {
            command.buttons & !mask
        }
    };
    match input {
        ScalarInput::Attack => Ok(ClientCommand {
            buttons: bit(ATTACK_BUTTON, value != 0.0),
            ..*command
        }),
        ScalarInput::Jump => match command.family {
            ClientFamily::Q1Netquake | ClientFamily::Q1Quakeworld => Ok(ClientCommand {
                buttons: bit(Q1_JUMP_BUTTON, value != 0.0),
                ..*command
            }),
            ClientFamily::Q2Rerelease => Ok(ClientCommand {
                buttons: bit(Q2R_JUMP_BUTTON, value != 0.0),
                ..*command
            }),
            ClientFamily::Q2Classic | ClientFamily::Q3 => {
                let up = if value == 0.0 {
                    command.up_move.min(0.0)
                } else {
                    command.up_move.max(10.0).max(move_scale(command.family))
                };
                Ok(ClientCommand {
                    up_move: up,
                    ..*command
                })
            }
        },
        ScalarInput::Impulse => {
            if value.fract() != 0.0 || value < 0.0 || value > f64::from(MAX_IMPULSE) {
                return Err(WorldError::BadClientCommand(
                    "Source impulse output must fit one byte".to_string(),
                ));
            }
            if carries_impulse(command.family) {
                Ok(ClientCommand {
                    impulse: value as i32,
                    ..*command
                })
            } else {
                Ok(*command)
            }
        }
        ScalarInput::ForwardMove => Ok(ClientCommand {
            forward_move: scale_move(value, command)?,
            ..*command
        }),
        ScalarInput::SideMove => {
            let scaled = scale_move(value, command)?;
            match command.family {
                ClientFamily::Q3 => Ok(ClientCommand {
                    right_move: scaled,
                    ..*command
                }),
                _ => Ok(ClientCommand {
                    side_move: scaled,
                    ..*command
                }),
            }
        }
        ScalarInput::UpMove => match command.family {
            ClientFamily::Q2Rerelease => {
                let buttons = (command.buttons & !(Q2R_JUMP_BUTTON | Q2R_DOWN_BUTTON))
                    | if value > 0.0 {
                        Q2R_JUMP_BUTTON
                    } else if value < 0.0 {
                        Q2R_DOWN_BUTTON
                    } else {
                        0
                    };
                Ok(ClientCommand { buttons, ..*command })
            }
            _ => Ok(ClientCommand {
                up_move: scale_move(value, command)?,
                ..*command
            }),
        },
    }
}

fn scale_move(value: f64, command: &ClientCommand) -> Result<f64, WorldError> {
    let scaled = value * move_scale(command.family);
    match command.family {
        ClientFamily::Q2Rerelease => {
            if !(scaled as f32).is_finite() {
                return Err(WorldError::BadClientCommand(
                    "Source movement output exceeds destination command ABI".to_string(),
                ));
            }
            Ok(scaled)
        }
        ClientFamily::Q3 => {
            if !(-127.0..=127.0).contains(&scaled) {
                return Err(WorldError::BadClientCommand(
                    "Source movement output exceeds destination command ABI".to_string(),
                ));
            }
            Ok(scaled.trunc())
        }
        _ => {
            if !(-32768.0..=32767.0).contains(&scaled) {
                return Err(WorldError::BadClientCommand(
                    "Source movement output exceeds destination command ABI".to_string(),
                ));
            }
            Ok(scaled.trunc())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_scales_match_donor() {
        assert_eq!(move_scale(ClientFamily::Q3), 127.0);
        assert_eq!(move_scale(ClientFamily::Q1Netquake), 320.0);
        assert_eq!(move_scale(ClientFamily::Q1Quakeworld), 320.0);
        assert_eq!(move_scale(ClientFamily::Q2Classic), 200.0);
        assert_eq!(move_scale(ClientFamily::Q2Rerelease), 200.0);
    }

    #[test]
    fn jump_detection_is_per_family() {
        let q1 = ClientCommand {
            buttons: 2,
            ..ClientCommand::zero(ClientFamily::Q1Netquake)
        };
        assert!(q1.jump_pressed());
        let q2r = ClientCommand {
            buttons: 8,
            ..ClientCommand::zero(ClientFamily::Q2Rerelease)
        };
        assert!(q2r.jump_pressed());
        let q3 = ClientCommand {
            up_move: 127.0,
            ..ClientCommand::zero(ClientFamily::Q3)
        };
        assert!(q3.jump_pressed());
        let q3_rest = ClientCommand {
            up_move: 9.0,
            ..ClientCommand::zero(ClientFamily::Q3)
        };
        assert!(!q3_rest.jump_pressed());
    }

    #[test]
    fn impulse_forced_zero_where_donor_has_none() {
        let q3 = ClientCommand {
            impulse: 7,
            ..ClientCommand::zero(ClientFamily::Q3)
        };
        assert_eq!(q3.effective_impulse(), 0);
        let q1 = ClientCommand {
            impulse: 7,
            ..ClientCommand::zero(ClientFamily::Q1Netquake)
        };
        assert_eq!(q1.effective_impulse(), 7);
        assert!(apply_scalar(&q3, ScalarInput::Impulse, 5.0).unwrap().impulse == 7);
        let applied = apply_scalar(&q1, ScalarInput::Impulse, 5.0).unwrap();
        assert_eq!(applied.impulse, 5);
        assert!(apply_scalar(&q1, ScalarInput::Impulse, 256.0).is_err());
        assert!(apply_scalar(&q1, ScalarInput::Impulse, 1.5).is_err());
    }

    #[test]
    fn scalar_moves_enforce_abi_ranges() {
        let q3 = ClientCommand::zero(ClientFamily::Q3);
        let applied = apply_scalar(&q3, ScalarInput::ForwardMove, 1.0).unwrap();
        assert_eq!(applied.forward_move, 127.0);
        assert!(apply_scalar(&q3, ScalarInput::ForwardMove, 2.0).is_err());
        let q1 = ClientCommand::zero(ClientFamily::Q1Netquake);
        let applied = apply_scalar(&q1, ScalarInput::SideMove, 0.5).unwrap();
        assert_eq!(applied.side_move, 160.0);
        assert_eq!(apply_scalar(&q3, ScalarInput::SideMove, 0.5).unwrap().right_move, 63.0);
        let q2r = ClientCommand::zero(ClientFamily::Q2Rerelease);
        let up = apply_scalar(&q2r, ScalarInput::UpMove, 1.0).unwrap();
        assert_eq!(up.buttons & Q2R_JUMP_BUTTON, Q2R_JUMP_BUTTON);
        let down = apply_scalar(&q2r, ScalarInput::UpMove, -1.0).unwrap();
        assert_eq!(down.buttons & Q2R_DOWN_BUTTON, Q2R_DOWN_BUTTON);
    }

    #[test]
    fn normalization_matches_input_values() {
        let command = ClientCommand {
            buttons: 3,
            forward_move: 160.0,
            side_move: -80.0,
            up_move: 0.0,
            ..ClientCommand::zero(ClientFamily::Q1Netquake)
        };
        let input = ClientInput::normalized(&command);
        assert!(input.attack && input.jump);
        assert_eq!(input.forward, 0.5);
        assert_eq!(input.lateral, -0.25);
        let q2r = ClientCommand {
            buttons: 16,
            ..ClientCommand::zero(ClientFamily::Q2Rerelease)
        };
        assert_eq!(ClientInput::normalized(&q2r).up, -1.0);
    }

    #[test]
    fn non_finite_scalar_is_rejected() {
        let q1 = ClientCommand::zero(ClientFamily::Q1Netquake);
        assert!(apply_scalar(&q1, ScalarInput::Attack, f64::NAN).is_err());
    }
}
