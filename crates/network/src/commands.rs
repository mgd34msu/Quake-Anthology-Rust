//! Protocol/ABI projections. Movement values retain the decoded command units;
//! The shared input builder owns speed scaling. Packet compression enters in R11.
use qa_core::primitives::{UserCmd, Vec3, WeaponId, buttons as b};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Q1Move {
    pub view_angles: Vec3,
    pub movement: [i16; 3],
    pub buttons: u8,
    pub impulse: u8,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QwCmd {
    pub msec: u8,
    pub view_angles: Vec3,
    pub movement: [i16; 3],
    pub buttons: u8,
    pub impulse: u8,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Q2Cmd {
    pub msec: u8,
    pub angles: [i16; 3],
    pub movement: [i16; 3],
    pub buttons: u8,
    pub impulse: u8,
    pub light_level: u8,
}
/// Rerelease game.h usercmd_t: native float movement/angles and byte buttons.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Q2RrCmd {
    pub msec: u8,
    pub buttons: u8,
    pub angles: Vec3,
    pub movement: [f32; 2],
    pub server_frame: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Q3Cmd {
    pub server_time: i32,
    pub angles: [i32; 3],
    pub movement: [i8; 3],
    pub buttons: u32,
    pub weapon: u8,
}

const Q1_BITS: [(u32, u32); 2] = [(1, b::ATTACK), (2, b::JUMP)];
const Q2_BITS: [(u32, u32); 3] = [(1, b::ATTACK), (2, b::USE), (128, b::ANY)];
const Q2_RR_BITS: [(u32, u32); 6] = [
    (1, b::ATTACK),
    (2, b::USE),
    (4, b::HOLSTER),
    (8, b::JUMP),
    (16, b::CROUCH),
    (128, b::ANY),
];
const Q3_BITS: [(u32, u32); 15] = [
    (1, b::ATTACK),
    (2, b::TALK),
    (4, b::USE),
    (8, b::GESTURE),
    (16, b::WALK),
    (32, b::AFFIRMATIVE),
    (64, b::NEGATIVE),
    (128, b::GETFLAG),
    (256, b::GUARDBASE),
    (512, b::PATROL),
    (1024, b::FOLLOWME),
    (2048, b::ANY),
    (1 << 12, b::EXTRA12),
    (1 << 13, b::EXTRA13),
    (1 << 14, b::EXTRA14),
];

fn from_buttons(value: u32, table: &[(u32, u32)]) -> u32 {
    table.iter().fold(0, |result, &(native, engine)| {
        if value & native != 0 {
            result | engine
        } else {
            result
        }
    })
}
fn to_buttons(value: u32, table: &[(u32, u32)]) -> u32 {
    table.iter().fold(0, |result, &(native, engine)| {
        if value & engine != 0 {
            result | native
        } else {
            result
        }
    })
}
fn angle_short(angle: f32) -> i16 {
    (angle * 65536.0 / 360.0) as i32 as i16
}
fn short_angle(angle: i32) -> f32 {
    angle as f32 * (360.0 / 65536.0)
}
fn vertical(command: &UserCmd, amount: f32) -> f32 {
    if command.movement[2] != 0.0 {
        command.movement[2]
    } else if command.buttons & b::JUMP != 0 {
        amount
    } else if command.buttons & b::CROUCH != 0 {
        -amount
    } else {
        0.0
    }
}

pub fn to_q1_move(command: &UserCmd) -> Q1Move {
    Q1Move {
        view_angles: command.view_angles,
        movement: command.movement.map(|value| value as i32 as i16),
        buttons: to_buttons(command.buttons, &Q1_BITS) as u8,
        impulse: command.impulse,
    }
}
pub fn from_q1_move(command: Q1Move, duration_ms: u16, server_time_ms: i32) -> UserCmd {
    UserCmd {
        duration_ms,
        server_time_ms,
        view_angles: command.view_angles,
        movement: command.movement.map(f32::from),
        buttons: from_buttons(u32::from(command.buttons), &Q1_BITS),
        impulse: command.impulse,
        ..UserCmd::default()
    }
}

pub fn to_qw_usercmd(command: &UserCmd) -> QwCmd {
    QwCmd {
        msec: command.duration_ms.min(255) as u8,
        view_angles: command.view_angles,
        movement: command.movement.map(|value| value as i32 as i16),
        buttons: to_buttons(command.buttons, &Q1_BITS) as u8,
        impulse: command.impulse,
    }
}
pub fn from_qw_usercmd(command: QwCmd, server_time_ms: i32) -> UserCmd {
    UserCmd {
        duration_ms: u16::from(command.msec),
        server_time_ms,
        view_angles: command.view_angles,
        movement: command.movement.map(f32::from),
        buttons: from_buttons(u32::from(command.buttons), &Q1_BITS),
        impulse: command.impulse,
        ..UserCmd::default()
    }
}

pub fn to_q2_usercmd(command: &UserCmd) -> Q2Cmd {
    let mut movement = command.movement;
    movement[2] = vertical(command, 200.0);
    Q2Cmd {
        msec: command.duration_ms.min(255) as u8,
        angles: command.view_angles.0.map(angle_short),
        movement: movement.map(|value| value as i32 as i16),
        buttons: to_buttons(command.buttons, &Q2_BITS) as u8,
        impulse: command.impulse,
        light_level: command.light_level,
    }
}
pub fn from_q2_usercmd(command: Q2Cmd, server_time_ms: i32) -> UserCmd {
    let vertical = if command.movement[2] > 0 {
        b::JUMP
    } else if command.movement[2] < 0 {
        b::CROUCH
    } else {
        0
    };
    UserCmd {
        duration_ms: u16::from(command.msec),
        server_time_ms,
        view_angles: Vec3(command.angles.map(|value| short_angle(i32::from(value)))),
        movement: command.movement.map(f32::from),
        buttons: from_buttons(u32::from(command.buttons), &Q2_BITS) | vertical,
        impulse: command.impulse,
        light_level: command.light_level,
        ..UserCmd::default()
    }
}

pub fn to_q2_rr_usercmd(command: &UserCmd, server_frame: u32) -> Q2RrCmd {
    let mut buttons = command.buttons;
    if command.movement[2] > 0.0 {
        buttons |= b::JUMP;
    }
    if command.movement[2] < 0.0 {
        buttons |= b::CROUCH;
    }
    Q2RrCmd {
        msec: command.duration_ms.min(255) as u8,
        buttons: to_buttons(buttons, &Q2_RR_BITS) as u8,
        angles: command.view_angles,
        movement: [command.movement[0], command.movement[1]],
        server_frame,
    }
}
pub fn from_q2_rr_usercmd(command: Q2RrCmd, server_time_ms: i32) -> UserCmd {
    UserCmd {
        duration_ms: u16::from(command.msec),
        server_time_ms,
        view_angles: command.angles,
        movement: [command.movement[0], command.movement[1], 0.0],
        buttons: from_buttons(u32::from(command.buttons), &Q2_RR_BITS),
        ..UserCmd::default()
    }
}

/// The registry supplies the original module's weapon number, not an ItemId cast.
pub fn to_q3_usercmd(command: &UserCmd, weapon: u8) -> Q3Cmd {
    let mut movement = command.movement;
    movement[2] = vertical(command, 127.0);
    Q3Cmd {
        server_time: command.server_time_ms,
        angles: command
            .view_angles
            .0
            .map(|value| i32::from(angle_short(value) as u16)),
        movement: movement.map(|value| value.clamp(-128.0, 127.0) as i8),
        buttons: to_buttons(command.buttons, &Q3_BITS),
        weapon,
    }
}
pub fn from_q3_usercmd(command: Q3Cmd, previous_server_time: i32, weapon: WeaponId) -> UserCmd {
    let vertical = if command.movement[2] >= 10 {
        b::JUMP
    } else if command.movement[2] < 0 {
        b::CROUCH
    } else {
        0
    };
    UserCmd {
        duration_ms: command
            .server_time
            .saturating_sub(previous_server_time)
            .clamp(0, i32::from(u16::MAX)) as u16,
        server_time_ms: command.server_time,
        view_angles: Vec3(command.angles.map(short_angle)),
        movement: command.movement.map(f32::from),
        buttons: from_buttons(command.buttons, &Q3_BITS) | vertical,
        weapon: Some(weapon),
        ..UserCmd::default()
    }
}
