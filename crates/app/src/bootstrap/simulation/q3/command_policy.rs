//! Q3 source command policy over translated foreign commands.
//!
//! Port of donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/q3/command-policy.ts`
//! (`applyQ3CommandPolicy`).
//!
//! The policy edits only the fields the Q3 source run translated (detected
//! by comparing the `before`/`after` source commands) while unchanged
//! foreign command units retain their original precision. Unified commands
//! reuse the `qa-net` actor command; source commands reuse the content Q3
//! user command.

use qa_content::q3::base::shared::player_state::UserCommand as Q3UserCommand;
use qa_net::common::commands::{ActorCommand, UserCommand};

/// Apply the Q3 source policy to a pending actor command.
///
/// `before`/`after` are the source commands bracketing the Q3 run,
/// `converted` is the translated selected command (which must keep the
/// original dialect), and `frozen` zeroes movement, buttons, and impulse
/// while clearing the arsenal weapon.
///
/// Panics with the donor message when the converted command changed the
/// selected command dialect.
#[must_use]
pub fn apply_q3_command_policy(
    pending: &ActorCommand,
    before: &Q3UserCommand,
    after: &Q3UserCommand,
    converted: &UserCommand,
    frozen: bool,
) -> ActorCommand {
    let mut envelope = pending.clone();
    if frozen {
        if let Some(arsenal) = envelope.arsenal.as_mut() {
            arsenal.weapon = None;
            arsenal.use_holdable = false;
        }
    }
    let original = &pending.command;
    if matches!(original, UserCommand::Q3 { .. }) {
        envelope.command = converted.clone();
        return envelope;
    }
    let moved_forward = before.forwardmove != after.forwardmove;
    let moved_side = before.rightmove != after.rightmove;
    let moved_up = before.upmove != after.upmove;
    let forward = |value: f64, changed: f64| {
        if frozen {
            0.0
        } else if moved_forward {
            changed
        } else {
            value
        }
    };
    let side = |value: f64, changed: f64| {
        if frozen {
            0.0
        } else if moved_side {
            changed
        } else {
            value
        }
    };
    let up = |value: f64, changed: f64| {
        if frozen {
            0.0
        } else if moved_up {
            changed
        } else {
            value
        }
    };
    let angle = |index: usize, value: f64, changed: f64| {
        let (before_angle, after_angle) = match index {
            0 => (before.angles.x, after.angles.x),
            1 => (before.angles.y, after.angles.y),
            _ => (before.angles.z, after.angles.z),
        };
        if before_angle == after_angle {
            value
        } else {
            changed
        }
    };
    let angles = |value: [f64; 3], changed: [f64; 3]| {
        [
            angle(0, value[0], changed[0]),
            angle(1, value[1], changed[1]),
            angle(2, value[2], changed[2]),
        ]
    };
    let buttons = |value: f64, changed: f64, jump_mask: i32| {
        if frozen {
            return 0.0;
        }
        let mask = 1 | if moved_up { jump_mask } else { 0 };
        f64::from((value as i32 & !mask) | (changed as i32 & mask))
    };
    let impulse = |value: f64| if frozen { 0.0 } else { value };
    let command = match (original, converted) {
        (
            UserCommand::Q1Netquake {
                acknowledged_server_time_seconds,
                view_angles,
                forward_move,
                side_move,
                up_move,
                buttons: original_buttons,
                impulse: original_impulse,
            },
            UserCommand::Q1Netquake {
                acknowledged_server_time_seconds: changed_time,
                view_angles: changed_angles,
                forward_move: changed_forward,
                side_move: changed_side,
                up_move: changed_up,
                buttons: changed_buttons,
                impulse: _,
            },
        ) => UserCommand::Q1Netquake {
            acknowledged_server_time_seconds: if before.server_time == after.server_time {
                *acknowledged_server_time_seconds
            } else {
                *changed_time
            },
            view_angles: angles(*view_angles, *changed_angles),
            forward_move: forward(*forward_move, *changed_forward),
            side_move: side(*side_move, *changed_side),
            up_move: up(*up_move, *changed_up),
            buttons: buttons(*original_buttons, *changed_buttons, 2),
            impulse: impulse(*original_impulse),
        },
        (
            UserCommand::Q1Quakeworld {
                milliseconds: _,
                angles: original_angles,
                forward_move,
                side_move,
                up_move,
                buttons: original_buttons,
                impulse: original_impulse,
            },
            UserCommand::Q1Quakeworld {
                milliseconds,
                angles: changed_angles,
                forward_move: changed_forward,
                side_move: changed_side,
                up_move: changed_up,
                buttons: changed_buttons,
                impulse: _,
            },
        ) => UserCommand::Q1Quakeworld {
            milliseconds: *milliseconds,
            angles: angles(*original_angles, *changed_angles),
            forward_move: forward(*forward_move, *changed_forward),
            side_move: side(*side_move, *changed_side),
            up_move: up(*up_move, *changed_up),
            buttons: buttons(*original_buttons, *changed_buttons, 0),
            impulse: impulse(*original_impulse),
        },
        (
            UserCommand::Q2Classic {
                milliseconds: _,
                angle_shorts,
                forward_move,
                side_move,
                up_move,
                buttons: original_buttons,
                impulse: original_impulse,
                light_level,
            },
            UserCommand::Q2Classic {
                milliseconds,
                angle_shorts: changed_shorts,
                forward_move: changed_forward,
                side_move: changed_side,
                up_move: changed_up,
                buttons: changed_buttons,
                impulse: _,
                light_level: _,
            },
        ) => UserCommand::Q2Classic {
            milliseconds: *milliseconds,
            angle_shorts: angles(*angle_shorts, *changed_shorts),
            forward_move: forward(*forward_move, *changed_forward),
            side_move: side(*side_move, *changed_side),
            up_move: up(*up_move, *changed_up),
            buttons: buttons(*original_buttons, *changed_buttons, 0),
            impulse: impulse(*original_impulse),
            light_level: *light_level,
        },
        (
            UserCommand::Q2Rerelease {
                milliseconds: _,
                angles: original_angles,
                forward_move,
                side_move,
                buttons: original_buttons,
                server_frame,
            },
            UserCommand::Q2Rerelease {
                milliseconds,
                angles: changed_angles,
                forward_move: changed_forward,
                side_move: changed_side,
                buttons: changed_buttons,
                server_frame: _,
            },
        ) => UserCommand::Q2Rerelease {
            milliseconds: *milliseconds,
            angles: angles(*original_angles, *changed_angles),
            forward_move: forward(*forward_move, *changed_forward),
            side_move: side(*side_move, *changed_side),
            buttons: buttons(*original_buttons, *changed_buttons, 8 | 16),
            server_frame: *server_frame,
        },
        _ => panic!("Q3 policy changed the selected command dialect"),
    };
    envelope.command = command;
    envelope
}

#[cfg(test)]
mod tests {
    use qa_core::identity::IdentityOwner;
    use qa_core::math::Vec3;
    use qa_net::common::commands::{ArsenalIntent, CommandSource};

    use super::*;

    fn source_command(forwardmove: i32, rightmove: i32, upmove: i32, angles: Vec3) -> Q3UserCommand {
        Q3UserCommand {
            server_time: 100,
            angles,
            buttons: 0,
            weapon: 2,
            forwardmove,
            rightmove,
            upmove,
        }
    }

    fn pending(command: UserCommand) -> ActorCommand {
        let owner = IdentityOwner::create("q3-policy-test").unwrap();
        ActorCommand {
            actor: owner.actor(0, 0),
            source: CommandSource::LocalSeat { seat: owner.seat(0) },
            sequence: 1,
            command,
            arsenal: Some(ArsenalIntent {
                provider: "q3:arsenal".to_string(),
                weapon: Some("q3:weapon/rocket".to_string()),
                use_holdable: true,
            }),
        }
    }

    fn quakeworld() -> UserCommand {
        UserCommand::Q1Quakeworld {
            milliseconds: 50.0,
            angles: [10.0, 20.0, 30.0],
            forward_move: 1.0,
            side_move: 2.0,
            up_move: 3.0,
            buttons: 32.0,
            impulse: 5.0,
        }
    }

    #[test]
    fn q3_commands_pass_through_converted() {
        let before = source_command(0, 0, 0, Vec3 { x: 0.0, y: 0.0, z: 0.0 });
        let pending = pending(UserCommand::Q3 {
            server_time_milliseconds: 1.0,
            angle_words: [0.0, 0.0, 0.0],
            buttons: 0.0,
            weapon: 0.0,
            forward_move: 0.0,
            right_move: 0.0,
            up_move: 0.0,
        });
        let converted = quakeworld();
        let applied = apply_q3_command_policy(&pending, &before, &before, &converted, false);
        assert_eq!(applied.command, converted);
    }

    #[test]
    fn unchanged_fields_keep_original_precision() {
        let angles = Vec3 { x: 1.0, y: 2.0, z: 3.0 };
        let before = source_command(0, 0, 0, angles);
        let after = source_command(9, 0, 0, Vec3 { x: 7.0, y: 2.0, z: 3.0 });
        let pending = pending(quakeworld());
        let converted = UserCommand::Q1Quakeworld {
            milliseconds: 60.0,
            angles: [70.0, 80.0, 90.0],
            forward_move: 11.0,
            side_move: 12.0,
            up_move: 13.0,
            buttons: 33.0,
            impulse: 6.0,
        };
        let applied = apply_q3_command_policy(&pending, &before, &after, &converted, false);
        match applied.command {
            UserCommand::Q1Quakeworld {
                milliseconds,
                angles,
                forward_move,
                side_move,
                up_move,
                buttons,
                impulse,
            } => {
                assert_eq!(milliseconds, 60.0);
                assert_eq!(angles, [70.0, 20.0, 30.0]);
                assert_eq!(forward_move, 11.0);
                assert_eq!(side_move, 2.0);
                assert_eq!(up_move, 3.0);
                assert_eq!(buttons, 33.0);
                assert_eq!(impulse, 5.0);
            }
            _ => panic!("dialect changed"),
        }
    }

    #[test]
    fn frozen_zeroes_movement_and_clears_arsenal() {
        let before = source_command(0, 0, 0, Vec3 { x: 0.0, y: 0.0, z: 0.0 });
        let after = source_command(9, 8, 7, Vec3 { x: 1.0, y: 1.0, z: 1.0 });
        let pending = pending(quakeworld());
        let converted = UserCommand::Q1Quakeworld {
            milliseconds: 60.0,
            angles: [70.0, 80.0, 90.0],
            forward_move: 11.0,
            side_move: 12.0,
            up_move: 13.0,
            buttons: 33.0,
            impulse: 6.0,
        };
        let applied = apply_q3_command_policy(&pending, &before, &after, &converted, true);
        match applied.command {
            UserCommand::Q1Quakeworld {
                forward_move,
                side_move,
                up_move,
                buttons,
                impulse,
                ..
            } => {
                assert_eq!(forward_move, 0.0);
                assert_eq!(side_move, 0.0);
                assert_eq!(up_move, 0.0);
                assert_eq!(buttons, 0.0);
                assert_eq!(impulse, 0.0);
            }
            _ => panic!("dialect changed"),
        }
        let arsenal = applied.arsenal.unwrap();
        assert_eq!(arsenal.weapon, None);
        assert!(!arsenal.use_holdable);
    }

    #[test]
    fn netquake_selects_server_time_and_jump_bit() {
        let before = source_command(0, 0, 0, Vec3 { x: 0.0, y: 0.0, z: 0.0 });
        let mut after = before;
        after.server_time = 200;
        after.upmove = 5;
        let original = UserCommand::Q1Netquake {
            acknowledged_server_time_seconds: 1.0,
            view_angles: [0.0, 0.0, 0.0],
            forward_move: 0.0,
            side_move: 0.0,
            up_move: 0.0,
            buttons: 0.0,
            impulse: 1.0,
        };
        let converted = UserCommand::Q1Netquake {
            acknowledged_server_time_seconds: 2.0,
            view_angles: [0.0, 0.0, 0.0],
            forward_move: 0.0,
            side_move: 0.0,
            up_move: 0.0,
            buttons: 3.0,
            impulse: 0.0,
        };
        let applied = apply_q3_command_policy(&pending(original), &before, &after, &converted, false);
        match applied.command {
            UserCommand::Q1Netquake {
                acknowledged_server_time_seconds,
                buttons,
                impulse,
                ..
            } => {
                assert_eq!(acknowledged_server_time_seconds, 2.0);
                assert_eq!(buttons, 3.0);
                assert_eq!(impulse, 1.0);
            }
            _ => panic!("dialect changed"),
        }
    }

    #[test]
    #[should_panic(expected = "Q3 policy changed the selected command dialect")]
    fn dialect_change_panics() {
        let before = source_command(0, 0, 0, Vec3 { x: 0.0, y: 0.0, z: 0.0 });
        let _ = apply_q3_command_policy(
            &pending(quakeworld()),
            &before,
            &before,
            &UserCommand::Q2Classic {
                milliseconds: 0.0,
                angle_shorts: [0.0, 0.0, 0.0],
                forward_move: 0.0,
                side_move: 0.0,
                up_move: 0.0,
                buttons: 0.0,
                impulse: 0.0,
                light_level: 0.0,
            },
            false,
        );
    }
}
