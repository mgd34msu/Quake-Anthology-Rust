//! Rerelease bot usercmd to per-dialect [`UserCommand`] translation.
//! Port of `src/app/bootstrap/simulation/bot-commands.ts`.

use qa_bots::behavior::rerelease::world::{
    BotUsercmdT, BOT_BUTTON_ATTACK, BOT_BUTTON_JUMP, BOT_BUTTON_USE,
};
use qa_bots::movement_contract::MovementKind;
use qa_net::common::commands::UserCommand;

#[allow(clippy::cast_possible_truncation, clippy::unnecessary_cast)]
fn angle_word(value: f64) -> f64 {
    ((value * 65536.0 / 360.0).trunc() as i32 & 0xFFFF) as f64
}

fn signed_byte(value: f64) -> f64 {
    (value * 127.0 / 320.0).trunc().clamp(-127.0, 127.0)
}

/// Bot movement is already expressed in source velocity units. Only Q3 uses signed command bytes.
#[allow(clippy::unnecessary_cast)]
pub fn rerelease_bot_command(
    source: &BotUsercmdT,
    dialect: MovementKind,
    milliseconds: f64,
    server_time: f64,
) -> UserCommand {
    let attack = source.buttons & BOT_BUTTON_ATTACK;
    let jump = (source.buttons & BOT_BUTTON_JUMP) != 0;
    let uses = (source.buttons & BOT_BUTTON_USE) != 0;
    let forward_move = source.forwardmove as f64;
    let side_move = source.sidemove as f64;
    let raw_up = source.upmove as f64;
    let up_move = if jump { raw_up.max(200.0) } else { raw_up };
    let angles = [
        source.view_angles.x as f64,
        source.view_angles.y as f64,
        source.view_angles.z as f64,
    ];
    match dialect {
        MovementKind::Q1Netquake => UserCommand::Q1Netquake {
            acknowledged_server_time_seconds: server_time / 1000.0,
            view_angles: angles,
            forward_move,
            side_move,
            up_move,
            buttons: (attack | if jump { 2 } else { 0 }) as f64,
            impulse: 0.0,
        },
        MovementKind::Q1Quakeworld => UserCommand::Q1Quakeworld {
            milliseconds,
            angles,
            forward_move,
            side_move,
            up_move,
            buttons: attack as f64,
            impulse: 0.0,
        },
        MovementKind::Q2Classic => UserCommand::Q2Classic {
            milliseconds,
            angle_shorts: [angle_word(angles[0]), angle_word(angles[1]), angle_word(angles[2])],
            forward_move,
            side_move,
            up_move,
            buttons: (attack | if uses { 2 } else { 0 }) as f64,
            impulse: 0.0,
            light_level: 0.0,
        },
        MovementKind::Q2Rerelease => UserCommand::Q2Rerelease {
            milliseconds,
            angles,
            forward_move,
            side_move,
            buttons: (attack | if uses { 2 } else { 0 } | if up_move > 0.0 { 8 } else if up_move < 0.0 { 16 } else { 0 }) as f64,
            server_frame: 0.0,
        },
        MovementKind::Q3 => UserCommand::Q3 {
            server_time_milliseconds: server_time,
            angle_words: [angle_word(angles[0]), angle_word(angles[1]), angle_word(angles[2])],
            forward_move: signed_byte(forward_move),
            right_move: signed_byte(side_move),
            up_move: if jump { 127.0 } else { signed_byte(raw_up) },
            buttons: attack as f64,
            weapon: 0.0,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_bots::behavior::rerelease::world::empty_usercmd;

    fn source() -> BotUsercmdT {
        let mut cmd = empty_usercmd();
        cmd.forwardmove = 320.0;
        cmd.sidemove = -160.0;
        cmd.upmove = 50.0;
        cmd
    }

    #[test]
    fn rerelease_buttons_combine_attack_use_and_up() {
        let mut cmd = source();
        cmd.buttons = BOT_BUTTON_ATTACK | BOT_BUTTON_USE;
        let UserCommand::Q2Rerelease { buttons, .. } =
            rerelease_bot_command(&cmd, MovementKind::Q2Rerelease, 16.0, 1000.0)
        else {
            panic!("expected q2-rerelease command");
        };
        assert_eq!(buttons, (BOT_BUTTON_ATTACK | 2 | 8) as f64);
    }

    #[test]
    fn jump_forces_minimum_up_move() {
        let mut cmd = source();
        cmd.buttons = BOT_BUTTON_JUMP;
        let UserCommand::Q2Classic { up_move, .. } =
            rerelease_bot_command(&cmd, MovementKind::Q2Classic, 16.0, 1000.0)
        else {
            panic!("expected q2-classic command");
        };
        assert_eq!(up_move, 200.0);
    }

    #[test]
    fn q3_scales_moves_to_signed_bytes() {
        let cmd = source();
        let UserCommand::Q3 { forward_move, right_move, up_move, .. } =
            rerelease_bot_command(&cmd, MovementKind::Q3, 16.0, 1000.0)
        else {
            panic!("expected q3 command");
        };
        assert_eq!((forward_move, right_move, up_move), (127.0, -63.0, 19.0));
    }

    #[test]
    fn netquake_reports_server_seconds_and_jump_bit() {
        let mut cmd = source();
        cmd.buttons = BOT_BUTTON_JUMP;
        let UserCommand::Q1Netquake { acknowledged_server_time_seconds, buttons, .. } =
            rerelease_bot_command(&cmd, MovementKind::Q1Netquake, 16.0, 2500.0)
        else {
            panic!("expected q1-netquake command");
        };
        assert_eq!(acknowledged_server_time_seconds, 2.5);
        assert_eq!(buttons, 2.0);
    }

    #[test]
    fn half_turn_maps_to_half_word() {
        assert_eq!(angle_word(180.0), 32768.0);
        assert_eq!(angle_word(360.0), 0.0);
    }
}
