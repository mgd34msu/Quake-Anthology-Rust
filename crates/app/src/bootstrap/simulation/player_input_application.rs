//! Player input application: aim resolution and command clocks.
//!
//! Port of donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/player-input-application.ts`
//! (`movementApplicationAim`, `movementApplicationFrame`).
//!
//! Movement entrypoints use native delta-angle conventions; the observer
//! also receives world aim.

use qa_core::math::{vec3, Vec3};
use qa_core::numeric::NumericOps;
use qa_core::time::{ClockProfile, FrameContext, SourceTime};
use qa_world::movement::q1::quakeworld::quake_world_command_slices;
use qa_world::movement::q1::types::{Q1MovementProfile, Q1MovementState, QwMovementProfile, QwMovementState};
use qa_world::movement::q2::types::{
    Q2MovementProfile, Q2MovementState, Q2RereleaseMovementProfile, Q2RereleaseMovementState, SrcVec3,
};
use qa_world::movement::q2::view::{classic_view_angles, rerelease_view_angles};
use qa_world::movement::q3::prediction::update_q3_prediction_view;
use qa_world::movement::q3::types::{Q3MovementProfile, Q3MovementState};
use qa_world::movement::types::{MovementError, UserCommand};
use thiserror::Error;

/// Mirror of `MovementState` from donor `src/contracts/movement.ts`
/// (canonical home: `qa_world::movement::types::MovementState`); unify
/// post-merge.
#[derive(Debug, Clone, PartialEq)]
pub enum MovementState {
    /// NetQuake state.
    Q1Netquake(Q1MovementState),
    /// QuakeWorld state.
    Q1Quakeworld(QwMovementState),
    /// Quake II classic state.
    Q2Classic(Q2MovementState),
    /// Quake II rerelease state.
    Q2Rerelease(Q2RereleaseMovementState),
    /// Quake III state.
    Q3(Q3MovementState),
}

/// Mirror of `MovementProfile` from donor `src/contracts/movement.ts`
/// (canonical home: `qa_world::movement::types::MovementProfile`); unify
/// post-merge.
#[derive(Debug, Clone, PartialEq)]
pub enum MovementProfile {
    /// NetQuake profile.
    Q1Netquake(Q1MovementProfile),
    /// QuakeWorld profile.
    Q1Quakeworld(QwMovementProfile),
    /// Quake II classic profile.
    Q2Classic(Q2MovementProfile),
    /// Quake II rerelease profile.
    Q2Rerelease(Q2RereleaseMovementProfile),
    /// Quake III profile.
    Q3(Q3MovementProfile),
}

impl MovementProfile {
    fn clock(&self) -> &ClockProfile {
        match self {
            MovementProfile::Q1Netquake(profile) => &profile.clock,
            MovementProfile::Q1Quakeworld(profile) => &profile.clock,
            MovementProfile::Q2Classic(profile) => &profile.clock,
            MovementProfile::Q2Rerelease(profile) => &profile.clock,
            MovementProfile::Q3(profile) => &profile.clock,
        }
    }
}

/// Input application failures.
#[derive(Debug, Error)]
pub enum InputApplicationError {
    /// Command and movement state belong to different families.
    #[error("input application command does not match movement state")]
    CommandStateMismatch,
    /// Q3 input application has no command clock.
    #[error("Q3 input application has no command clock")]
    NoCommandClock,
    /// QuakeWorld command slicing failed.
    #[error("quakeworld command slicing failed: {0:?}")]
    Slices(#[from] MovementError),
}

fn to_src(value: Vec3) -> SrcVec3 {
    [f64::from(value.x), f64::from(value.y), f64::from(value.z)]
}

fn from_src(value: SrcVec3) -> Vec3 {
    vec3(value[0] as f32, value[1] as f32, value[2] as f32)
}

/// Resolve world aim for a command against its movement state.
pub fn movement_application_aim(
    command: &UserCommand,
    state: &MovementState,
    health: f64,
    numeric: &NumericOps,
) -> Result<Vec3, InputApplicationError> {
    match (command, state) {
        (UserCommand::Q1Netquake(command), _) => Ok(command.view_angles),
        (UserCommand::Q1Quakeworld(command), _) => Ok(command.angles),
        (UserCommand::Q2Rerelease(command), MovementState::Q2Rerelease(state)) => {
            let mut view: SrcVec3 = [0.0, 0.0, 0.0];
            rerelease_view_angles(
                &mut view,
                to_src(command.angles),
                to_src(state.delta_angles),
                state.flags,
                numeric,
            );
            Ok(from_src(view))
        }
        (UserCommand::Q2Classic(command), MovementState::Q2Classic(state)) => {
            let mut view: SrcVec3 = [0.0, 0.0, 0.0];
            classic_view_angles(
                &mut view,
                command.angle_shorts,
                state.delta_angle_shorts,
                state.flags,
                numeric,
            );
            Ok(from_src(view))
        }
        (UserCommand::Q3(command), MovementState::Q3(state)) => {
            Ok(update_q3_prediction_view(state, health, command).view_angles)
        }
        _ => Err(InputApplicationError::CommandStateMismatch),
    }
}

/// Resolve the frame clock a command executes under.
pub fn movement_application_frame(
    command: &UserCommand,
    state: &MovementState,
    profile: &MovementProfile,
    frame: FrameContext,
) -> Result<FrameContext, InputApplicationError> {
    match command {
        UserCommand::Q1Netquake(_) => Ok(frame),
        UserCommand::Q3(command) => {
            let MovementState::Q3(state) = state else {
                return Err(InputApplicationError::NoCommandClock);
            };
            let milliseconds = (command.server_time_milliseconds - state.command_time_milliseconds).clamp(0, 1000);
            Ok(FrameContext {
                elapsed: SourceTime::Milliseconds(milliseconds),
                ..frame
            })
        }
        UserCommand::Q1Quakeworld(command) => match profile.clock() {
            ClockProfile::Q1Quakeworld {
                maximum_command_milliseconds,
            } => {
                let mut milliseconds = 0;
                for slice in quake_world_command_slices(command, *maximum_command_milliseconds)? {
                    milliseconds += slice.milliseconds;
                }
                Ok(FrameContext {
                    elapsed: SourceTime::Milliseconds(milliseconds),
                    ..frame
                })
            }
            _ => Ok(frame),
        },
        UserCommand::Q2Classic(command) => Ok(FrameContext {
            elapsed: SourceTime::Milliseconds(command.milliseconds),
            ..frame
        }),
        UserCommand::Q2Rerelease(command) => Ok(FrameContext {
            elapsed: SourceTime::Milliseconds(command.milliseconds),
            ..frame
        }),
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::ProviderId;
    use qa_core::numeric::{NumericOps, Q2_DONOR_PROFILE};
    use qa_world::movement::types::{
        Q1UserCommand, Q2RereleaseUserCommand, Q2UserCommand, Q3UserCommand, QwUserCommand, TraceHit,
    };
    use qa_world::movement::Q1MovementParameters;

    use super::*;

    fn numeric() -> NumericOps {
        NumericOps::select(Q2_DONOR_PROFILE).expect("numeric")
    }

    fn test_frame() -> FrameContext {
        FrameContext {
            frame: 7,
            time: SourceTime::Milliseconds(700),
            elapsed: SourceTime::Milliseconds(100),
            phase: qa_core::time::FramePhase::EntityPhysics,
        }
    }

    fn q1_state() -> MovementState {
        MovementState::Q1Netquake(Q1MovementState {
            origin: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            angles: vec3(0.0, 0.0, 0.0),
            old_origin: vec3(0.0, 0.0, 0.0),
            angular_velocity: vec3(0.0, 0.0, 0.0),
            view_angles: vec3(0.0, 0.0, 0.0),
            punch_angles: vec3(0.0, 0.0, 0.0),
            move_type: 0,
            flags: 0,
            ground: TraceHit::None,
            water_level: 0,
            water_type: -1,
            teleport_time_seconds: 0.0,
            water_jump_direction: vec3(0.0, 0.0, 0.0),
            ideal_pitch: 0.0,
            fix_angle: false,
            health: 100.0,
        })
    }

    fn qw_profile(maximum: f64) -> MovementProfile {
        MovementProfile::Q1Quakeworld(QwMovementProfile {
            id: ProviderId::new("q1", "test"),
            clock: ClockProfile::Q1Quakeworld {
                maximum_command_milliseconds: maximum,
            },
            numeric: Q2_DONOR_PROFILE,
            parameters: Q1MovementParameters {
                gravity: 800.0,
                stop_speed: 100.0,
                max_speed: 320.0,
                spectator_max_speed: 500.0,
                accelerate: 10.0,
                air_accelerate: 1.0,
                water_accelerate: 10.0,
                friction: 4.0,
                water_friction: 1.0,
                entity_gravity: 1.0,
            },
        })
    }

    #[test]
    fn netquake_aim_is_command_view() {
        let command = UserCommand::Q1Netquake(Q1UserCommand {
            acknowledged_server_time_seconds: 1.0,
            view_angles: vec3(10.0, 20.0, 0.0),
            forward_move: 0.0,
            side_move: 0.0,
            up_move: 0.0,
            buttons: 0,
            impulse: 0,
        });
        let numeric = numeric();
        assert_eq!(
            movement_application_aim(&command, &q1_state(), 100.0, &numeric).expect("aim"),
            vec3(10.0, 20.0, 0.0)
        );
        // NetQuake ignores the movement state entirely.
        let qw = MovementState::Q1Quakeworld(QwMovementState {
            origin: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            angles: vec3(0.0, 0.0, 0.0),
            old_buttons: 0,
            water_jump_time_seconds: 0.0,
            dead: false,
            spectator: 0,
            ground: TraceHit::None,
        });
        assert_eq!(
            movement_application_aim(&command, &qw, 100.0, &numeric).expect("aim"),
            vec3(10.0, 20.0, 0.0)
        );
    }

    #[test]
    fn quakeworld_aim_is_command_angles() {
        let command = UserCommand::Q1Quakeworld(QwUserCommand {
            milliseconds: 50,
            angles: vec3(5.0, 90.0, 0.0),
            forward_move: 0.0,
            side_move: 0.0,
            up_move: 0.0,
            buttons: 0,
            impulse: 0,
        });
        let numeric = numeric();
        assert_eq!(
            movement_application_aim(&command, &q1_state(), 100.0, &numeric).expect("aim"),
            vec3(5.0, 90.0, 0.0)
        );
    }

    #[test]
    fn q2_aim_matches_native_view() {
        let numeric = numeric();
        let command = UserCommand::Q2Classic(Q2UserCommand {
            milliseconds: 50,
            angle_shorts: [4096, 8192, 0],
            forward_move: 0.0,
            side_move: 0.0,
            up_move: 0.0,
            buttons: 0,
            impulse: 0,
            light_level: 0,
        });
        let state = MovementState::Q2Classic(Q2MovementState {
            move_type: 0,
            origin_eighths: [0, 0, 0],
            velocity_eighths: [0, 0, 0],
            flags: 0,
            time_eight_milliseconds: 0,
            gravity: 800.0,
            delta_angle_shorts: [0, 0, 0],
        });
        let mut expected: SrcVec3 = [0.0, 0.0, 0.0];
        classic_view_angles(&mut expected, [4096, 8192, 0], [0, 0, 0], 0, &numeric);
        assert_eq!(
            movement_application_aim(&command, &state, 100.0, &numeric).expect("aim"),
            from_src(expected)
        );
        let rerelease = UserCommand::Q2Rerelease(Q2RereleaseUserCommand {
            milliseconds: 50,
            angles: vec3(10.0, 20.0, 30.0),
            forward_move: 0.0,
            side_move: 0.0,
            buttons: 0,
            server_frame: 0,
        });
        let rstate = MovementState::Q2Rerelease(Q2RereleaseMovementState {
            move_type: 0,
            origin: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            flags: 0,
            time_milliseconds: 0,
            gravity: 800.0,
            delta_angles: vec3(0.0, 0.0, 0.0),
            view_height: 22.0,
        });
        let mut rexpected: SrcVec3 = [0.0, 0.0, 0.0];
        rerelease_view_angles(
            &mut rexpected,
            to_src(vec3(10.0, 20.0, 30.0)),
            to_src(vec3(0.0, 0.0, 0.0)),
            0,
            &numeric,
        );
        assert_eq!(
            movement_application_aim(&rerelease, &rstate, 100.0, &numeric).expect("aim"),
            from_src(rexpected)
        );
    }

    #[test]
    fn mismatched_command_and_state_fails() {
        let numeric = numeric();
        let command = UserCommand::Q2Classic(Q2UserCommand {
            milliseconds: 50,
            angle_shorts: [0, 0, 0],
            forward_move: 0.0,
            side_move: 0.0,
            up_move: 0.0,
            buttons: 0,
            impulse: 0,
            light_level: 0,
        });
        assert!(matches!(
            movement_application_aim(&command, &q1_state(), 100.0, &numeric),
            Err(InputApplicationError::CommandStateMismatch)
        ));
    }

    #[test]
    fn q3_frame_clamps_command_clock() {
        let state = MovementState::Q3(Q3MovementState {
            command_time_milliseconds: 900,
            movement_type: 0,
            bob_cycle: 0,
            movement_flags: 0,
            movement_time_milliseconds: 0,
            origin: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            gravity: 800.0,
            speed: 0.0,
            delta_angle_words: [0, 0, 0],
            movement_direction: 0,
            grapple_point: vec3(0.0, 0.0, 0.0),
            flags: 0,
            view_angles: vec3(0.0, 0.0, 0.0),
            view_height: 26.0,
            ground: TraceHit::None,
            predictable_event_sequence: 0,
            jump_pad: None,
            movement_frame: 0,
            jump_pad_frame: 0,
        });
        let profile = MovementProfile::Q3(Q3MovementProfile {
            id: ProviderId::new("q3", "test"),
            clock: ClockProfile::Q3 {
                server_frame_milliseconds: 100.0,
                fixed_movement_milliseconds: None,
            },
            numeric: Q2_DONOR_PROFILE,
            product: qa_world::movement::q3::types::Q3Product::BaseQ3,
            fixed_milliseconds: None,
            no_footsteps: false,
        });
        let command = UserCommand::Q3(Q3UserCommand {
            server_time_milliseconds: 950,
            angle_words: [0, 0, 0],
            buttons: 0,
            weapon: 0,
            forward_move: 0,
            right_move: 0,
            up_move: 0,
        });
        let frame = movement_application_frame(&command, &state, &profile, test_frame()).expect("frame");
        assert_eq!(frame.elapsed, SourceTime::Milliseconds(50));
        let future = UserCommand::Q3(Q3UserCommand {
            server_time_milliseconds: 5000,
            ..match command {
                UserCommand::Q3(command) => command,
                _ => unreachable!(),
            }
        });
        let frame = movement_application_frame(&future, &state, &profile, test_frame()).expect("frame");
        assert_eq!(frame.elapsed, SourceTime::Milliseconds(1000));
        assert!(matches!(
            movement_application_frame(&command, &q1_state(), &profile, test_frame()),
            Err(InputApplicationError::NoCommandClock)
        ));
    }

    #[test]
    fn quakeworld_frame_sums_slices() {
        let command = UserCommand::Q1Quakeworld(QwUserCommand {
            milliseconds: 100,
            angles: vec3(0.0, 0.0, 0.0),
            forward_move: 0.0,
            side_move: 0.0,
            up_move: 0.0,
            buttons: 0,
            impulse: 0,
        });
        let frame = movement_application_frame(&command, &q1_state(), &qw_profile(50.0), test_frame()).expect("frame");
        assert_eq!(frame.elapsed, SourceTime::Milliseconds(100));
        // A non-QuakeWorld clock leaves the frame untouched.
        let netquake = MovementProfile::Q1Netquake(Q1MovementProfile {
            id: ProviderId::new("q1", "test"),
            clock: ClockProfile::Q1Netquake {
                minimum_frame_seconds: 0.0,
                maximum_frame_seconds: 1.0,
                fixed_frame_seconds: None,
            },
            numeric: Q2_DONOR_PROFILE,
            edition: qa_world::movement::q1::types::Q1Edition::Classic,
            parameters: Q1MovementParameters {
                gravity: 800.0,
                stop_speed: 100.0,
                max_speed: 320.0,
                spectator_max_speed: 500.0,
                accelerate: 10.0,
                air_accelerate: 1.0,
                water_accelerate: 10.0,
                friction: 4.0,
                water_friction: 1.0,
                entity_gravity: 1.0,
            },
            edge_friction: 2.0,
            no_clip_angle_hack: false,
        });
        let base = test_frame();
        let frame = movement_application_frame(&command, &q1_state(), &netquake, base).expect("frame");
        assert_eq!(frame, base);
    }

    #[test]
    fn netquake_frame_is_untouched_and_q2_passes_milliseconds() {
        let base = test_frame();
        let netquake = UserCommand::Q1Netquake(Q1UserCommand {
            acknowledged_server_time_seconds: 1.0,
            view_angles: vec3(0.0, 0.0, 0.0),
            forward_move: 0.0,
            side_move: 0.0,
            up_move: 0.0,
            buttons: 0,
            impulse: 0,
        });
        let frame = movement_application_frame(&netquake, &q1_state(), &qw_profile(50.0), base).expect("frame");
        assert_eq!(frame, base);
        let classic = UserCommand::Q2Classic(Q2UserCommand {
            milliseconds: 77,
            angle_shorts: [0, 0, 0],
            forward_move: 0.0,
            side_move: 0.0,
            up_move: 0.0,
            buttons: 0,
            impulse: 0,
            light_level: 0,
        });
        let frame = movement_application_frame(&classic, &q1_state(), &qw_profile(50.0), base).expect("frame");
        assert_eq!(frame.elapsed, SourceTime::Milliseconds(77));
    }
}
