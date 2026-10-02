//! Conversions between selected commands and Q3 source commands.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/q3-commands.ts`
//! (`relativeQ3SourceCommand`, `q3SourceCommand`, `q3CommandForControls`,
//! `selectedQ3Command`, `relativeMovementCommand`).

use qa_content::contract::ExecutableRecipe;
use qa_content::q3::base::shared::definitions::Product;
use qa_content::q3::base::shared::player_state::UserCommand as Q3SourceUserCommand;
use qa_core::math::{vec3, Vec3};
use qa_net::common::commands::{
    ActorCommand, CommandSource, MovementDialect as SelectedDialect, UserCommand as SelectedUserCommand,
};
use qa_world::movement::types::{
    ArsenalState, MovementDialect, Q1UserCommand, Q2RereleaseUserCommand, Q2UserCommand, Q3UserCommand, QwUserCommand,
    UserCommand, WeaponState,
};

use super::arsenal::selected::MovementState;
use super::arsenal_intent::{resolve_q3_arsenal_controls, ArsenalIntentError};
use super::prediction::types::MovementPredictionProfile;

/// Command angle space, mirroring donor `ActorCommand["angleSpace"]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandAngleSpace {
    /// Absolute unified aim.
    Absolute,
    /// Source-relative words.
    SourceRelative,
}

/// Mirror of MovementPlayer from donor src/app/bootstrap/simulation/players.ts (canonical home: crate::bootstrap::simulation::players); unify post-merge.
#[derive(Debug, Clone)]
pub struct MovementPlayer {
    /// Selected movement profile.
    pub profile: MovementPredictionProfile,
    /// Current arsenal.
    pub arsenal: ArsenalState,
    /// Executable recipe.
    pub recipe: ExecutableRecipe,
}

/// Errors building a Q3 source command.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Q3CommandError {
    /// Arsenal intent does not fit the selected Q3 product.
    #[error(transparent)]
    Intent(#[from] ArsenalIntentError),
}

/// Foreign local input owns absolute aim; Q3 commands already contain
/// source-relative words.
pub fn relative_q3_source_command(
    source: &CommandSource,
    dialect: SelectedDialect,
    command: &Q3SourceUserCommand,
    delta: &Vec3,
    angle_space: Option<CommandAngleSpace>,
) -> Q3SourceUserCommand {
    let absolute = angle_space == Some(CommandAngleSpace::Absolute)
        || angle_space.is_none() && matches!(source, CommandSource::LocalSeat { .. }) && dialect != SelectedDialect::Q3;
    if !absolute {
        return *command;
    }
    let mut relative = *command;
    relative.angles = vec3(
        command.angles.x - delta.x,
        command.angles.y - delta.y,
        command.angles.z - delta.z,
    );
    relative
}

pub(crate) fn command_buttons(command: &UserCommand) -> i32 {
    match command {
        UserCommand::Q1Netquake(command) => command.buttons,
        UserCommand::Q1Quakeworld(command) => command.buttons,
        UserCommand::Q2Classic(command) => command.buttons,
        UserCommand::Q2Rerelease(command) => command.buttons,
        UserCommand::Q3(command) => command.buttons,
    }
}

/// Source client policy receives command units independently of the selected
/// PMove input.
pub fn q3_source_command(
    input: &ActorCommand,
    player: &MovementPlayer,
    milliseconds: f64,
    native_weapon: i32,
) -> Result<Q3SourceUserCommand, Q3CommandError> {
    let command = selected_to_movement(&input.command);
    if matches!(player.arsenal.state, WeaponState::Q3 { .. }) {
        let product = if player.recipe.map.entities.content.as_str().contains("missionpack") {
            Product::Missionpack
        } else {
            Product::Baseq3
        };
        let controls = resolve_q3_arsenal_controls(&player.arsenal, input.arsenal.as_ref(), &command, product)?;
        Ok(q3_command_for_controls(
            &command,
            milliseconds,
            controls.requested_weapon,
            controls.use_holdable,
        ))
    } else {
        let use_holdable = input
            .arsenal
            .as_ref()
            .map(|arsenal| arsenal.use_holdable)
            .unwrap_or(matches!(command, UserCommand::Q3(_)) && command_buttons(&command) & 4 != 0);
        Ok(q3_command_for_controls(
            &command,
            milliseconds,
            native_weapon,
            use_holdable,
        ))
    }
}

/// Convert a protocol command to its movement-kernel form. The donor has one
/// `UserCommand` shape; Rust splits protocol (`qa_net`) from movement
/// (`qa_world`) precision, so the `ActorCommand` boundary converts once here.
pub(crate) fn selected_to_movement(command: &SelectedUserCommand) -> UserCommand {
    match command {
        SelectedUserCommand::Q1Netquake {
            acknowledged_server_time_seconds,
            view_angles,
            forward_move,
            side_move,
            up_move,
            buttons,
            impulse,
        } => UserCommand::Q1Netquake(Q1UserCommand {
            acknowledged_server_time_seconds: *acknowledged_server_time_seconds,
            view_angles: vec3(view_angles[0] as f32, view_angles[1] as f32, view_angles[2] as f32),
            forward_move: *forward_move,
            side_move: *side_move,
            up_move: *up_move,
            buttons: *buttons as i32,
            impulse: *impulse as i32,
        }),
        SelectedUserCommand::Q1Quakeworld {
            milliseconds,
            angles,
            forward_move,
            side_move,
            up_move,
            buttons,
            impulse,
        } => UserCommand::Q1Quakeworld(QwUserCommand {
            milliseconds: *milliseconds as i32,
            angles: vec3(angles[0] as f32, angles[1] as f32, angles[2] as f32),
            forward_move: *forward_move,
            side_move: *side_move,
            up_move: *up_move,
            buttons: *buttons as i32,
            impulse: *impulse as i32,
        }),
        SelectedUserCommand::Q2Classic {
            milliseconds,
            angle_shorts,
            forward_move,
            side_move,
            up_move,
            buttons,
            impulse,
            light_level,
        } => UserCommand::Q2Classic(Q2UserCommand {
            milliseconds: *milliseconds as i32,
            angle_shorts: [angle_shorts[0] as i32, angle_shorts[1] as i32, angle_shorts[2] as i32],
            forward_move: *forward_move,
            side_move: *side_move,
            up_move: *up_move,
            buttons: *buttons as i32,
            impulse: *impulse as i32,
            light_level: *light_level as i32,
        }),
        SelectedUserCommand::Q2Rerelease {
            milliseconds,
            angles,
            forward_move,
            side_move,
            buttons,
            server_frame,
        } => UserCommand::Q2Rerelease(Q2RereleaseUserCommand {
            milliseconds: *milliseconds as i32,
            angles: vec3(angles[0] as f32, angles[1] as f32, angles[2] as f32),
            forward_move: *forward_move,
            side_move: *side_move,
            buttons: *buttons as i32,
            server_frame: *server_frame as i32,
        }),
        SelectedUserCommand::Q3 {
            server_time_milliseconds,
            angle_words,
            buttons,
            weapon,
            forward_move,
            right_move,
            up_move,
        } => UserCommand::Q3(Q3UserCommand {
            server_time_milliseconds: *server_time_milliseconds as i32,
            angle_words: [angle_words[0] as i32, angle_words[1] as i32, angle_words[2] as i32],
            buttons: *buttons as i32,
            weapon: *weapon as i32,
            forward_move: *forward_move as i32,
            right_move: *right_move as i32,
            up_move: *up_move as i32,
        }),
    }
}

fn angle_word(degrees: f32) -> i32 {
    (f64::from(degrees) * 65536.0 / 360.0).trunc() as i32 & 0xffff
}

fn move_axis(value: f64, divisor: f64) -> i32 {
    (value * 127.0 / divisor).clamp(-127.0, 127.0).trunc() as i32
}

/// Build a Q3 source command from resolved arsenal controls.
pub fn q3_command_for_controls(
    command: &UserCommand,
    milliseconds: f64,
    requested_weapon: i32,
    use_holdable: bool,
) -> Q3SourceUserCommand {
    let buttons = match command {
        UserCommand::Q3(command) => command.buttons & !4,
        _ => command_buttons(command) & 1,
    } | if use_holdable { 4 } else { 0 };
    if let UserCommand::Q3(command) = command {
        return Q3SourceUserCommand {
            server_time: command.server_time_milliseconds,
            angles: vec3(
                command.angle_words[0] as f32,
                command.angle_words[1] as f32,
                command.angle_words[2] as f32,
            ),
            buttons,
            weapon: requested_weapon,
            forwardmove: command.forward_move,
            rightmove: command.right_move,
            upmove: command.up_move,
        };
    }
    let (angles, forward_move, side_move, up_move, buttons_word) = match command {
        UserCommand::Q2Classic(command) => (
            vec3(
                command.angle_shorts[0] as f32,
                command.angle_shorts[1] as f32,
                command.angle_shorts[2] as f32,
            ),
            command.forward_move,
            command.side_move,
            command.up_move,
            command.buttons,
        ),
        UserCommand::Q1Netquake(command) => (
            vec3(
                angle_word(command.view_angles.x) as f32,
                angle_word(command.view_angles.y) as f32,
                angle_word(command.view_angles.z) as f32,
            ),
            command.forward_move,
            command.side_move,
            command.up_move,
            command.buttons,
        ),
        UserCommand::Q1Quakeworld(command) => (
            vec3(
                angle_word(command.angles.x) as f32,
                angle_word(command.angles.y) as f32,
                angle_word(command.angles.z) as f32,
            ),
            command.forward_move,
            command.side_move,
            command.up_move,
            command.buttons,
        ),
        UserCommand::Q2Rerelease(command) => (
            vec3(
                angle_word(command.angles.x) as f32,
                angle_word(command.angles.y) as f32,
                angle_word(command.angles.z) as f32,
            ),
            command.forward_move,
            command.side_move,
            0.0,
            command.buttons,
        ),
        UserCommand::Q3(_) => unreachable!("q3 commands return above"),
    };
    let divisor = match command {
        UserCommand::Q1Netquake(_) | UserCommand::Q1Quakeworld(_) => 320.0,
        _ => 200.0,
    };
    let upmove = match command {
        UserCommand::Q2Rerelease(_) if buttons_word & 8 != 0 => 127,
        UserCommand::Q2Rerelease(_) if buttons_word & 16 != 0 => -127,
        UserCommand::Q2Rerelease(_) => 0,
        UserCommand::Q1Netquake(_) | UserCommand::Q1Quakeworld(_) if buttons_word & 2 != 0 => 127,
        _ => move_axis(up_move, divisor),
    };
    Q3SourceUserCommand {
        server_time: milliseconds.trunc() as i32,
        angles,
        buttons,
        weapon: requested_weapon,
        forwardmove: move_axis(forward_move, divisor),
        rightmove: move_axis(side_move, divisor),
        upmove,
    }
}

/// Source spawn/inactivity frames can supply commands without a new transport
/// packet.
pub fn selected_q3_command(
    command: &Q3SourceUserCommand,
    profile: &MovementPredictionProfile,
    milliseconds: i32,
) -> UserCommand {
    let angles = vec3(
        (f64::from(command.angles.x) * 360.0 / 65536.0) as f32,
        (f64::from(command.angles.y) * 360.0 / 65536.0) as f32,
        (f64::from(command.angles.z) * 360.0 / 65536.0) as f32,
    );
    let forward_move = f64::from(command.forwardmove) * 320.0 / 127.0;
    let side_move = f64::from(command.rightmove) * 320.0 / 127.0;
    let up_move = f64::from(command.upmove) * 320.0 / 127.0;
    match profile.dialect() {
        MovementDialect::Q1Netquake => UserCommand::Q1Netquake(Q1UserCommand {
            acknowledged_server_time_seconds: f64::from(command.server_time) / 1000.0,
            view_angles: angles,
            forward_move,
            side_move,
            up_move,
            buttons: (command.buttons & 1) | if command.upmove > 0 { 2 } else { 0 },
            impulse: 0,
        }),
        MovementDialect::Q1Quakeworld => UserCommand::Q1Quakeworld(QwUserCommand {
            milliseconds,
            angles,
            forward_move,
            side_move,
            up_move,
            buttons: command.buttons & 1,
            impulse: 0,
        }),
        MovementDialect::Q2Classic => UserCommand::Q2Classic(Q2UserCommand {
            milliseconds,
            angle_shorts: [
                command.angles.x as i32,
                command.angles.y as i32,
                command.angles.z as i32,
            ],
            forward_move: f64::from(command.forwardmove) * 200.0 / 127.0,
            side_move: f64::from(command.rightmove) * 200.0 / 127.0,
            up_move: f64::from(command.upmove) * 200.0 / 127.0,
            buttons: command.buttons & 1,
            impulse: 0,
            light_level: 0,
        }),
        MovementDialect::Q2Rerelease => UserCommand::Q2Rerelease(Q2RereleaseUserCommand {
            milliseconds,
            angles,
            forward_move: f64::from(command.forwardmove) * 200.0 / 127.0,
            side_move: f64::from(command.rightmove) * 200.0 / 127.0,
            buttons: (command.buttons & 1)
                | if command.upmove > 0 {
                    8
                } else if command.upmove < 0 {
                    16
                } else {
                    0
                },
            server_frame: 0,
        }),
        MovementDialect::Q3 => UserCommand::Q3(Q3UserCommand {
            server_time_milliseconds: command.server_time,
            angle_words: [
                command.angles.x as i32,
                command.angles.y as i32,
                command.angles.z as i32,
            ],
            forward_move: command.forwardmove,
            right_move: command.rightmove,
            up_move: command.upmove,
            buttons: command.buttons,
            weapon: command.weapon,
        }),
    }
}

/// Input carrying a movement command plus its angle space, mirroring donor
/// `Pick<ActorCommand, "command" | "angleSpace">`.
pub trait RelativeMovementCommandInput: Clone {
    /// Movement command.
    fn command(&self) -> &UserCommand;
    /// Aim space of the command angles.
    fn angle_space(&self) -> Option<CommandAngleSpace>;
    /// Rebuild the input with source-relative angles.
    fn with_relative_command(&self, command: UserCommand) -> Self;
}

/// PMove adds its own delta angles; absolute unified aim crosses that boundary
/// once.
pub fn relative_movement_command<T: RelativeMovementCommandInput>(input: &T, state: &MovementState) -> T {
    if input.angle_space() != Some(CommandAngleSpace::Absolute) {
        return input.clone();
    }
    let command = input.command();
    match (command, state) {
        (UserCommand::Q3(command), MovementState::Q3(state)) => {
            let mut relative = *command;
            relative.angle_words = [
                command.angle_words[0] - state.delta_angle_words[0],
                command.angle_words[1] - state.delta_angle_words[1],
                command.angle_words[2] - state.delta_angle_words[2],
            ];
            input.with_relative_command(UserCommand::Q3(relative))
        }
        (UserCommand::Q2Classic(command), MovementState::Q2Classic(state)) => {
            let mut relative = *command;
            relative.angle_shorts = [
                command.angle_shorts[0] - state.delta_angle_shorts[0],
                command.angle_shorts[1] - state.delta_angle_shorts[1],
                command.angle_shorts[2] - state.delta_angle_shorts[2],
            ];
            input.with_relative_command(UserCommand::Q2Classic(relative))
        }
        (UserCommand::Q2Rerelease(command), MovementState::Q2Rerelease(state)) => {
            let mut relative = *command;
            relative.angles = vec3(
                command.angles.x - state.delta_angles.x,
                command.angles.y - state.delta_angles.y,
                command.angles.z - state.delta_angles.z,
            );
            input.with_relative_command(UserCommand::Q2Rerelease(relative))
        }
        _ => input.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_world::movement::q3::types::Q3MovementState;

    fn source_command() -> Q3SourceUserCommand {
        Q3SourceUserCommand {
            server_time: 1200,
            angles: vec3(100.0, 200.0, 300.0),
            buttons: 5,
            weapon: 7,
            forwardmove: 64,
            rightmove: -32,
            upmove: 0,
        }
    }

    #[test]
    fn local_seat_commands_keep_source_words() {
        let owner = qa_core::identity::IdentityOwner::create("q3cmd").unwrap();
        let source = CommandSource::LocalSeat { seat: owner.seat(0) };
        let relative = relative_q3_source_command(
            &source,
            SelectedDialect::Q1Netquake,
            &source_command(),
            &vec3(10.0, 20.0, 30.0),
            None,
        );
        assert_eq!(relative.angles, vec3(90.0, 180.0, 270.0));
        let kept = relative_q3_source_command(
            &source,
            SelectedDialect::Q3,
            &source_command(),
            &vec3(10.0, 20.0, 30.0),
            None,
        );
        assert_eq!(kept.angles, vec3(100.0, 200.0, 300.0));
    }

    #[test]
    fn controls_build_source_commands_per_dialect() {
        let q3 = UserCommand::Q3(Q3UserCommand {
            server_time_milliseconds: 900,
            angle_words: [1, 2, 3],
            buttons: 5,
            weapon: 2,
            forward_move: 10,
            right_move: 20,
            up_move: 30,
        });
        let built = q3_command_for_controls(&q3, 901.7, 8, true);
        assert_eq!(built.server_time, 900);
        assert_eq!(built.weapon, 8);
        assert_eq!(built.buttons, 5);
        assert_eq!(built.forwardmove, 10);

        let qw = UserCommand::Q1Quakeworld(QwUserCommand {
            milliseconds: 50,
            angles: vec3(90.0, 0.0, 0.0),
            forward_move: 320.0,
            side_move: 0.0,
            up_move: 0.0,
            buttons: 3,
            impulse: 0,
        });
        let built = q3_command_for_controls(&qw, 44.9, 2, false);
        assert_eq!(built.server_time, 44);
        assert_eq!(built.angles.x as i32, 90 * 65536 / 360);
        assert_eq!(built.forwardmove, 127);
        assert_eq!(built.upmove, 127);
        assert_eq!(built.buttons, 1);
    }

    #[test]
    fn delta_angles_cross_the_boundary_once() {
        let state = MovementState::Q3(Q3MovementState {
            command_time_milliseconds: 0,
            movement_type: 0,
            bob_cycle: 0,
            movement_flags: 0,
            movement_time_milliseconds: 0,
            origin: Vec3::default(),
            velocity: Vec3::default(),
            gravity: 800.0,
            speed: 0.0,
            delta_angle_words: [10, 20, 30],
            movement_direction: 0,
            grapple_point: Vec3::default(),
            flags: 0,
            view_angles: Vec3::default(),
            view_height: 0.0,
            ground: qa_world::movement::types::TraceHit::None,
            predictable_event_sequence: 0,
            jump_pad: None,
            movement_frame: 0,
            jump_pad_frame: 0,
        });
        let input = crate::bootstrap::simulation::prediction::types::PredictionCommand {
            angle_space: Some(CommandAngleSpace::Absolute),
            sequence: 7,
            time_milliseconds: 100.0,
            command: UserCommand::Q3(Q3UserCommand {
                server_time_milliseconds: 100,
                angle_words: [110, 220, 330],
                buttons: 0,
                weapon: 1,
                forward_move: 0,
                right_move: 0,
                up_move: 0,
            }),
            arsenal: None,
        };
        let relative = relative_movement_command(&input, &state);
        assert_eq!(relative.angle_space, Some(CommandAngleSpace::SourceRelative));
        match relative.command {
            UserCommand::Q3(command) => assert_eq!(command.angle_words, [100, 200, 300]),
            _ => panic!("dialect changed"),
        }
        let again = relative_movement_command(&relative, &state);
        assert_eq!(again, relative);
    }
}
