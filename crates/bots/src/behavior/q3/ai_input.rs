//! Bot view and command conversion from `src/bots/behavior/q3/ai-input.ts`
//! (`game/ai_main.c`: `BotChangeViewAngles`, `BotInputToUserCommand`,
//! `BotUpdateInput`).
//!
//! View angles ease toward the ideal with the character's view factor
//! and per-second cap; the detached action cell then converts to a
//! user command with delta-angle encoding and direction projection.

use qa_core::math::{angle_vectors, dot3, Vec3};

use crate::behavior::library::actions::{BotActionFlag, BotInput};
use crate::behavior::library::character::Characteristic;
use crate::behavior::q3::ai_state::{BotState, BotUserCommand, CommandButtons};

/// Wrap an angle to `[0, 360)`.
#[must_use]
pub fn angle_mod(angle: f32) -> f32 {
    angle.rem_euclid(360.0)
}

/// Signed angle difference in `(-180, 180]`.
#[must_use]
pub fn bot_angle_difference(first: f32, second: f32) -> f32 {
    let mut difference = first - second;
    if first > second {
        if difference > 180.0 {
            difference -= 360.0;
        }
    } else if difference < -180.0 {
        difference += 360.0;
    }
    difference
}

/// Step an angle toward ideal, limited by speed.
#[must_use]
pub fn bot_change_view_angle(angle: f32, ideal: f32, speed: f32) -> f32 {
    let angle = angle_mod(angle);
    let ideal = angle_mod(ideal);
    if angle == ideal {
        return angle;
    }
    let mut travel = ideal - angle;
    if ideal > angle {
        if travel > 180.0 {
            travel -= 360.0;
        }
    } else if travel < -180.0 {
        travel += 360.0;
    }
    let step = if travel > 0.0 {
        travel.min(speed)
    } else {
        travel.max(-speed)
    };
    angle_mod(angle + step)
}

/// Ease view angles toward the ideal for a think step.
pub fn bot_change_view_angles(
    characters: &crate::behavior::library::character::BotCharacterLibrary,
    state: &mut BotState,
    think_time: f32,
    challenge: bool,
) {
    if state.ideal_viewangles.x > 180.0 {
        state.ideal_viewangles.x -= 360.0;
    }
    let factor = if state.enemy >= 0 {
        characters.bounded_float(state.character, Characteristic::VIEW_FACTOR, 0.01, 1.0)
    } else {
        0.05
    };
    let mut maximum = if state.enemy >= 0 {
        characters.bounded_float(state.character, Characteristic::VIEW_MAX_CHANGE, 1.0, 1800.0)
    } else {
        360.0
    };
    if maximum < 240.0 {
        maximum = 240.0;
    }
    maximum *= think_time;
    for axis in [0, 1] {
        let (mut angle, ideal, mut velocity) = if axis == 0 {
            (state.viewangles.x, state.ideal_viewangles.x, state.viewangle_speed.x)
        } else {
            (state.viewangles.y, state.ideal_viewangles.y, state.viewangle_speed.y)
        };
        if challenge {
            let difference = bot_angle_difference(angle, ideal).abs() as i32 as f32;
            let mut speed = difference * factor;
            if speed > maximum {
                speed = maximum;
            }
            angle = bot_change_view_angle(angle, ideal, speed);
        } else {
            angle = angle_mod(angle);
            let ideal = angle_mod(ideal);
            let desired = bot_angle_difference(angle, ideal) * factor;
            velocity += velocity - desired;
            if velocity > 180.0 {
                velocity = maximum;
            }
            if velocity < -180.0 {
                velocity = -maximum;
            }
            let mut speed = velocity;
            if speed > maximum {
                speed = maximum;
            }
            if speed < -maximum {
                speed = -maximum;
            }
            angle = angle_mod(angle + speed);
            velocity *= 0.45 * (1.0 - factor);
            if axis == 0 {
                state.ideal_viewangles.x = ideal;
            } else {
                state.ideal_viewangles.y = ideal;
            }
        }
        if axis == 0 {
            state.viewangles.x = angle;
            state.viewangle_speed.x = velocity;
        } else {
            state.viewangles.y = angle;
            state.viewangle_speed.y = velocity;
        }
    }
    if state.viewangles.x > 180.0 {
        state.viewangles.x -= 360.0;
    }
}

fn signed_byte(value: i32) -> i32 {
    ((value << 24) >> 24).clamp(-128, 127)
}

fn command_angle(angle: f32, delta: f32) -> i32 {
    let encoded = ((angle * 65536.0 / 360.0) as i32) & 65535;
    (((encoded - delta as i32) << 16) >> 16).clamp(-32768, 32767)
}

/// Convert a detached action cell to a user command.
pub fn bot_input_to_user_command(input: &BotInput, command: &mut BotUserCommand, delta_angles: Vec3, time: i32) {
    let mut flags = input.action_flags;
    if flags & BotActionFlag::DELAYED_JUMP != 0 {
        flags = (flags | BotActionFlag::JUMP) & !BotActionFlag::DELAYED_JUMP;
    }
    command.server_time = time;
    command.buttons = 0;
    if flags & (BotActionFlag::RESPAWN | BotActionFlag::ATTACK) != 0 {
        command.buttons = CommandButtons::ATTACK;
    }
    const BUTTONS: [(i32, i32); 10] = [
        (BotActionFlag::TALK, CommandButtons::TALK),
        (BotActionFlag::GESTURE, CommandButtons::GESTURE),
        (BotActionFlag::USE, CommandButtons::USE_HOLDABLE),
        (BotActionFlag::WALK, CommandButtons::WALKING),
        (BotActionFlag::AFFIRMATIVE, CommandButtons::AFFIRMATIVE),
        (BotActionFlag::NEGATIVE, CommandButtons::NEGATIVE),
        (BotActionFlag::GET_FLAG, CommandButtons::GETFLAG),
        (BotActionFlag::GUARD_BASE, CommandButtons::GUARDBASE),
        (BotActionFlag::PATROL, CommandButtons::PATROL),
        (BotActionFlag::FOLLOW_ME, CommandButtons::FOLLOWME),
    ];
    for (action, button) in BUTTONS {
        if flags & action != 0 {
            command.buttons |= button;
        }
    }
    command.weapon = input.weapon & 255;
    command.angles = Vec3 {
        x: command_angle(input.view_angles.x, delta_angles.x) as f32,
        y: command_angle(input.view_angles.y, delta_angles.y) as f32,
        z: command_angle(input.view_angles.z, delta_angles.z) as f32,
    };
    let angles = Vec3 {
        x: if input.direction.z != 0.0 {
            input.view_angles.x
        } else {
            0.0
        },
        y: input.view_angles.y,
        z: 0.0,
    };
    let vectors = angle_vectors(angles);
    let speed = input.speed * 127.0 / 400.0;
    command.forwardmove = signed_byte((dot3(vectors.forward, input.direction) * speed) as i32);
    command.rightmove = signed_byte((dot3(vectors.right, input.direction) * speed) as i32);
    let up = (vectors.forward.z.abs() * input.direction.z * speed) as i32;
    command.upmove = signed_byte(up);
    if flags & BotActionFlag::MOVE_FORWARD != 0 {
        command.forwardmove = signed_byte(command.forwardmove + 127);
    }
    if flags & BotActionFlag::MOVE_BACK != 0 {
        command.forwardmove = signed_byte(command.forwardmove - 127);
    }
    if flags & BotActionFlag::MOVE_LEFT != 0 {
        command.rightmove = signed_byte(command.rightmove - 127);
    }
    if flags & BotActionFlag::MOVE_RIGHT != 0 {
        command.rightmove = signed_byte(command.rightmove + 127);
    }
    if flags & BotActionFlag::JUMP != 0 {
        command.upmove = signed_byte(command.upmove + 127);
    }
    if flags & BotActionFlag::CROUCH != 0 {
        command.upmove = signed_byte(command.upmove - 127);
    }
}

/// Add delta angles to the view angles.
pub fn bot_add_delta_angles(state: &mut BotState) {
    let delta = state.cur_ps.delta_angles;
    state.viewangles = Vec3 {
        x: angle_mod(state.viewangles.x + delta.x * (360.0 / 65536.0)),
        y: angle_mod(state.viewangles.y + delta.y * (360.0 / 65536.0)),
        z: angle_mod(state.viewangles.z + delta.z * (360.0 / 65536.0)),
    };
}

/// Subtract delta angles from the view angles.
pub fn bot_subtract_delta_angles(state: &mut BotState) {
    let delta = state.cur_ps.delta_angles;
    state.viewangles = Vec3 {
        x: angle_mod(state.viewangles.x - delta.x * (360.0 / 65536.0)),
        y: angle_mod(state.viewangles.y - delta.y * (360.0 / 65536.0)),
        z: angle_mod(state.viewangles.z - delta.z * (360.0 / 65536.0)),
    };
}
