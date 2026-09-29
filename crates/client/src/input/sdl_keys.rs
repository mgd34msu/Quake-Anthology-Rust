//! SDL key translation and event timing.
//!
//! Donor provenance: `src/input/sdl-keys.ts` (`sdlGameKey`,
//! `sdlEventTime`), ported from `unix/linux_glimp.c` `XLateKey`.

use super::KeyCode;

/// SDL scancode to Quake key for flagged keycodes.
fn special_key(code: i32) -> i32 {
    match code {
        72 => KeyCode::Pause as i32,
        73 => KeyCode::Insert as i32,
        74 => KeyCode::Home as i32,
        75 => KeyCode::PageUp as i32,
        77 => KeyCode::End as i32,
        78 => KeyCode::PageDown as i32,
        79 => KeyCode::Right as i32,
        80 => KeyCode::Left as i32,
        81 => KeyCode::Down as i32,
        82 => KeyCode::Up as i32,
        84 => KeyCode::KeypadSlash as i32,
        85 => 42,
        86 => KeyCode::KeypadMinus as i32,
        87 => KeyCode::KeypadPlus as i32,
        88 => KeyCode::KeypadEnter as i32,
        89 => KeyCode::KeypadEnd as i32,
        90 => KeyCode::KeypadDown as i32,
        91 => KeyCode::KeypadPageDown as i32,
        92 => KeyCode::KeypadLeft as i32,
        93 => KeyCode::Keypad5 as i32,
        94 => KeyCode::KeypadRight as i32,
        95 => KeyCode::KeypadHome as i32,
        96 => KeyCode::KeypadUp as i32,
        97 => KeyCode::KeypadPageUp as i32,
        98 => KeyCode::KeypadInsert as i32,
        99 => KeyCode::KeypadDelete as i32,
        103 => 61,
        116 => KeyCode::Control as i32,
        205 => KeyCode::Space as i32,
        224 | 228 => KeyCode::Control as i32,
        225 | 229 => KeyCode::Shift as i32,
        226 | 227 | 230 | 231 => KeyCode::Alt as i32,
        _ => 0,
    }
}

/// `XLookupString` ASCII control conversion.
fn control_byte(byte: i32) -> i32 {
    if (64..127).contains(&byte) || byte == 32 {
        byte & 31
    } else if byte == 50 {
        0
    } else if (51..=55).contains(&byte) {
        byte - 24
    } else if byte == 56 {
        127
    } else if byte == 47 {
        31
    } else {
        byte
    }
}

/// Modifier bits.
pub mod sdl_modifier {
    /// Either control key.
    pub const CONTROL: u16 = 0xc0;
    /// Either shift key.
    pub const SHIFT: u16 = 0x3;
    /// Num-lock.
    pub const NUM_LOCK: u16 = 0x1000;
}

/// Translate an SDL keycode to a Quake key (`sdlGameKey`).
///
/// Returns 0 for keys the engine ignores.
#[must_use]
pub fn sdl_game_key(keycode: i32, modifiers: u16) -> i32 {
    if keycode & 0x4000_0000 != 0 {
        let code = keycode & !0x4000_0000;
        if (58..=69).contains(&code) {
            return KeyCode::F1 as i32 + code - 58;
        }
        if code == 93 && modifiers & sdl_modifier::NUM_LOCK != 0 {
            return if modifiers & sdl_modifier::CONTROL != 0 {
                29
            } else {
                53
            };
        }
        return special_key(code);
    }
    match keycode {
        8 => KeyCode::Backspace as i32,
        127 => KeyCode::Delete as i32,
        33 => 49,
        64 => 50,
        35 => 51,
        36 => 52,
        37 => 53,
        94 => 54,
        38 => 55,
        42 => 56,
        40 => 57,
        41 => 48,
        178 => 126,
        9 => {
            if modifiers & sdl_modifier::SHIFT != 0 {
                0
            } else {
                KeyCode::Tab as i32
            }
        }
        13 => KeyCode::Enter as i32,
        27 => KeyCode::Escape as i32,
        32 => KeyCode::Space as i32,
        _ => {
            let byte = if modifiers & sdl_modifier::CONTROL != 0 {
                control_byte(keycode)
            } else {
                keycode
            };
            if (65..=90).contains(&byte) {
                byte + 32
            } else if (1..=26).contains(&byte) {
                byte + 96
            } else if (1..=255).contains(&byte) {
                byte
            } else {
                0
            }
        }
    }
}

/// `Sys_XTimeToSysTime` 30 ms subframe correction (`sdlEventTime`).
#[must_use]
pub fn sdl_event_time(timestamp: u32, ticks: u32, now: i64, subframe: bool) -> i64 {
    let age = ticks.wrapping_sub(timestamp) as i32;
    if subframe && (0..=30).contains(&age) {
        now.wrapping_sub(i64::from(age))
    } else {
        now
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_printable_and_control_keys() {
        assert_eq!(sdl_game_key(65, 0), 97);
        assert_eq!(sdl_game_key(97, 0), 97);
        assert_eq!(sdl_game_key(33, 0), 49);
        assert_eq!(sdl_game_key(8, 0), KeyCode::Backspace as i32);
        assert_eq!(sdl_game_key(127, 0), KeyCode::Delete as i32);
        assert_eq!(sdl_game_key(9, 0), KeyCode::Tab as i32);
        assert_eq!(sdl_game_key(9, 1), 0);
        assert_eq!(sdl_game_key(13, 0), KeyCode::Enter as i32);
        assert_eq!(sdl_game_key(27, 0), KeyCode::Escape as i32);
        assert_eq!(sdl_game_key(32, 0), KeyCode::Space as i32);
        assert_eq!(sdl_game_key(0x4000_0000 + 82, 0), KeyCode::Up as i32);
        assert_eq!(sdl_game_key(0x4000_0000 + 58, 0), KeyCode::F1 as i32);
        assert_eq!(sdl_game_key(0x4000_0000 + 69, 0), KeyCode::F12 as i32);
        assert_eq!(sdl_game_key(0x4000_0000 + 57, 0), 0);
        assert_eq!(sdl_game_key(0x4000_0000 + 200, 0), 0);
        assert_eq!(
            sdl_game_key(0x4000_0000 + 93, sdl_modifier::NUM_LOCK),
            53
        );
        assert_eq!(
            sdl_game_key(
                0x4000_0000 + 93,
                sdl_modifier::NUM_LOCK | sdl_modifier::CONTROL
            ),
            29
        );
        assert_eq!(sdl_game_key(65, sdl_modifier::CONTROL), 97);
        assert_eq!(sdl_game_key(0, 0), 0);
        assert_eq!(sdl_game_key(256, 0), 0);
    }

    #[test]
    fn corrects_fresh_subframe_timestamps() {
        assert_eq!(sdl_event_time(1000, 1010, 5000, true), 4990);
        assert_eq!(sdl_event_time(1000, 1031, 5000, true), 5000);
        assert_eq!(sdl_event_time(1010, 1000, 5000, true), 5000);
        assert_eq!(sdl_event_time(1000, 1010, 5000, false), 5000);
    }
}
