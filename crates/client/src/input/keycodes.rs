//! Key names and VM-facing key numbers.
//!
//! Donor provenance: `src/input/keys.ts` (`stringToKeynum`,
//! `keynumToString`, ported from Quake III `cl_keys.c`) and the
//! `sourceKeyNumber` table in `src/input/bindings.ts`.

use qa_core::cmd::source_command_text;

use super::KeyCode;

/// Key-catcher bitmask (`KeyCatcher`).
pub mod key_catcher {
    /// Console catcher.
    pub const CONSOLE: u32 = 1;
    /// UI catcher.
    pub const UI: u32 = 2;
    /// Message catcher.
    pub const MESSAGE: u32 = 4;
    /// Cgame catcher.
    pub const CGAME: u32 = 8;
}

/// Named keys in donor table order (scan order matters for aliases).
const KEY_NAMES: &[(&str, i32)] = &[
    ("TAB", KeyCode::Tab as i32),
    ("ENTER", KeyCode::Enter as i32),
    ("ESCAPE", KeyCode::Escape as i32),
    ("SPACE", KeyCode::Space as i32),
    ("BACKSPACE", KeyCode::Backspace as i32),
    ("UPARROW", KeyCode::Up as i32),
    ("DOWNARROW", KeyCode::Down as i32),
    ("LEFTARROW", KeyCode::Left as i32),
    ("RIGHTARROW", KeyCode::Right as i32),
    ("ALT", KeyCode::Alt as i32),
    ("CTRL", KeyCode::Control as i32),
    ("SHIFT", KeyCode::Shift as i32),
    ("COMMAND", KeyCode::Command as i32),
    ("CAPSLOCK", KeyCode::CapsLock as i32),
    ("F1", KeyCode::F1 as i32),
    ("F2", KeyCode::F2 as i32),
    ("F3", KeyCode::F3 as i32),
    ("F4", KeyCode::F4 as i32),
    ("F5", KeyCode::F5 as i32),
    ("F6", KeyCode::F6 as i32),
    ("F7", KeyCode::F7 as i32),
    ("F8", KeyCode::F8 as i32),
    ("F9", KeyCode::F9 as i32),
    ("F10", KeyCode::F10 as i32),
    ("F11", KeyCode::F11 as i32),
    ("F12", KeyCode::F12 as i32),
    ("INS", KeyCode::Insert as i32),
    ("DEL", KeyCode::Delete as i32),
    ("PGDN", KeyCode::PageDown as i32),
    ("PGUP", KeyCode::PageUp as i32),
    ("HOME", KeyCode::Home as i32),
    ("END", KeyCode::End as i32),
    ("MOUSE1", KeyCode::Mouse1 as i32),
    ("MOUSE2", KeyCode::Mouse2 as i32),
    ("MOUSE3", KeyCode::Mouse3 as i32),
    ("MOUSE4", KeyCode::Mouse4 as i32),
    ("MOUSE5", KeyCode::Mouse5 as i32),
    ("MWHEELUP", KeyCode::MouseWheelUp as i32),
    ("MWHEELDOWN", KeyCode::MouseWheelDown as i32),
    ("JOY1", KeyCode::Joy1 as i32),
    ("JOY2", KeyCode::Joy2 as i32),
    ("JOY3", KeyCode::Joy3 as i32),
    ("JOY4", KeyCode::Joy4 as i32),
    ("JOY5", KeyCode::Joy5 as i32),
    ("JOY6", KeyCode::Joy6 as i32),
    ("JOY7", KeyCode::Joy7 as i32),
    ("JOY8", KeyCode::Joy8 as i32),
    ("JOY9", KeyCode::Joy9 as i32),
    ("JOY10", KeyCode::Joy10 as i32),
    ("JOY11", KeyCode::Joy11 as i32),
    ("JOY12", KeyCode::Joy12 as i32),
    ("JOY13", KeyCode::Joy13 as i32),
    ("JOY14", KeyCode::Joy14 as i32),
    ("JOY15", KeyCode::Joy15 as i32),
    ("JOY16", KeyCode::Joy16 as i32),
    ("JOY17", KeyCode::Joy17 as i32),
    ("JOY18", KeyCode::Joy18 as i32),
    ("JOY19", KeyCode::Joy19 as i32),
    ("JOY20", KeyCode::Joy20 as i32),
    ("JOY21", KeyCode::Joy21 as i32),
    ("JOY22", KeyCode::Joy22 as i32),
    ("JOY23", KeyCode::Joy23 as i32),
    ("JOY24", KeyCode::Joy24 as i32),
    ("JOY25", KeyCode::Joy25 as i32),
    ("JOY26", KeyCode::Joy26 as i32),
    ("JOY27", KeyCode::Joy27 as i32),
    ("JOY28", KeyCode::Joy28 as i32),
    ("JOY29", KeyCode::Joy29 as i32),
    ("JOY30", KeyCode::Joy30 as i32),
    ("JOY31", KeyCode::Joy31 as i32),
    ("JOY32", KeyCode::Joy32 as i32),
    ("AUX1", KeyCode::Aux1 as i32),
    ("AUX2", KeyCode::Aux2 as i32),
    ("AUX3", KeyCode::Aux3 as i32),
    ("AUX4", KeyCode::Aux4 as i32),
    ("AUX5", KeyCode::Aux5 as i32),
    ("AUX6", KeyCode::Aux6 as i32),
    ("AUX7", KeyCode::Aux7 as i32),
    ("AUX8", KeyCode::Aux8 as i32),
    ("AUX9", KeyCode::Aux9 as i32),
    ("AUX10", KeyCode::Aux10 as i32),
    ("AUX11", KeyCode::Aux11 as i32),
    ("AUX12", KeyCode::Aux12 as i32),
    ("AUX13", KeyCode::Aux13 as i32),
    ("AUX14", KeyCode::Aux14 as i32),
    ("AUX15", KeyCode::Aux15 as i32),
    ("AUX16", KeyCode::Aux16 as i32),
    ("AUX17", 256),
    ("AUX18", 257),
    ("AUX19", 258),
    ("AUX20", 259),
    ("AUX21", 260),
    ("AUX22", 261),
    ("AUX23", 262),
    ("AUX24", 263),
    ("AUX25", 264),
    ("AUX26", 265),
    ("AUX27", 266),
    ("AUX28", 267),
    ("AUX29", 268),
    ("AUX30", 269),
    ("AUX31", 270),
    ("AUX32", 271),
    ("KP_HOME", KeyCode::KeypadHome as i32),
    ("KP_UPARROW", KeyCode::KeypadUp as i32),
    ("KP_PGUP", KeyCode::KeypadPageUp as i32),
    ("KP_LEFTARROW", KeyCode::KeypadLeft as i32),
    ("KP_5", KeyCode::Keypad5 as i32),
    ("KP_RIGHTARROW", KeyCode::KeypadRight as i32),
    ("KP_END", KeyCode::KeypadEnd as i32),
    ("KP_DOWNARROW", KeyCode::KeypadDown as i32),
    ("KP_PGDN", KeyCode::KeypadPageDown as i32),
    ("KP_ENTER", KeyCode::KeypadEnter as i32),
    ("KP_INS", KeyCode::KeypadInsert as i32),
    ("KP_DEL", KeyCode::KeypadDelete as i32),
    ("KP_SLASH", KeyCode::KeypadSlash as i32),
    ("KP_MINUS", KeyCode::KeypadMinus as i32),
    ("KP_PLUS", KeyCode::KeypadPlus as i32),
    ("KP_NUMLOCK", KeyCode::KeypadNumLock as i32),
    ("KP_STAR", KeyCode::KeypadStar as i32),
    ("KP_EQUALS", KeyCode::KeypadEquals as i32),
    ("PAUSE", KeyCode::Pause as i32),
    ("SEMICOLON", 59),
];

fn hex_nibble(code: u8) -> i32 {
    if code.is_ascii_digit() {
        i32::from(code) - 48
    } else if (b'a'..=b'f').contains(&code) {
        i32::from(code) - 87
    } else {
        0
    }
}

/// Map a key name to its key number (`stringToKeynum`).
///
/// Returns -1 for missing, empty, or unknown names.
#[must_use]
pub fn string_to_keynum(value: Option<&str>) -> i32 {
    let Some(value) = value else {
        return -1;
    };
    let Ok(text) = source_command_text(value) else {
        return -1;
    };
    if text.is_empty() {
        return -1;
    }
    let mut chars = text.chars();
    if let (Some(first), None) = (chars.next(), chars.next()) {
        let unit = first as u32;
        return if unit < 128 {
            unit as i32
        } else {
            unit as i32 - 256
        };
    }
    let bytes = text.as_bytes();
    if text.starts_with("0x") && text.len() == 4 {
        return hex_nibble(bytes[2]) * 16 + hex_nibble(bytes[3]);
    }
    for (name, number) in KEY_NAMES {
        if text.eq_ignore_ascii_case(name) {
            return *number;
        }
    }
    -1
}

/// Map a key number to its display name (`keynumToString`).
#[must_use]
pub fn keynum_to_string(key: i32) -> String {
    if key == -1 {
        return "<KEY NOT FOUND>".to_string();
    }
    if !(0..=271).contains(&key) {
        return "<OUT OF RANGE>".to_string();
    }
    if key > 32 && key < 127 && key != 34 && key != 59 {
        return char::from_u32(key as u32).map_or_else(
            || format!("0x{key:02x}"),
            |value| value.to_string(),
        );
    }
    for (name, number) in KEY_NAMES {
        if key == *number {
            return (*name).to_string();
        }
    }
    format!("0x{key:02x}")
}

/// Game family for VM-facing key numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyFamily {
    /// Quake I.
    Q1,
    /// Quake II.
    Q2,
    /// Quake III.
    Q3,
}

/// VM-facing source key value (`sourceKeyNumber`).
///
/// Returns `None` for keys the family cannot represent.
#[must_use]
pub fn source_key_number(key: i32, family: KeyFamily) -> Option<i32> {
    if family == KeyFamily::Q3 {
        return (0..=255).contains(&key).then_some(key);
    }
    if key < 128 {
        return Some(key);
    }
    let up = KeyCode::Up as i32;
    let shift = KeyCode::Shift as i32;
    let insert = KeyCode::Insert as i32;
    let end = KeyCode::End as i32;
    let f1 = KeyCode::F1 as i32;
    let f12 = KeyCode::F12 as i32;
    let mouse1 = KeyCode::Mouse1 as i32;
    let mouse3 = KeyCode::Mouse3 as i32;
    let joy1 = KeyCode::Joy1 as i32;
    let joy4 = KeyCode::Joy4 as i32;
    let aux1 = KeyCode::Aux1 as i32;
    let aux16 = KeyCode::Aux16 as i32;
    if (up..=shift).contains(&key) {
        return Some(key - 4);
    }
    if (insert..=end).contains(&key) {
        return Some(key + 8);
    }
    if (f1..=f12).contains(&key) {
        return Some(key - 10);
    }
    if key == KeyCode::Pause as i32 {
        return Some(255);
    }
    if (mouse1..=mouse3).contains(&key) {
        return Some(key - mouse1 + 200);
    }
    if (joy1..=joy4).contains(&key) {
        return Some(key - joy1 + 203);
    }
    if (aux1..=aux16).contains(&key) {
        return Some(key - aux1 + 207);
    }
    if (256..=271).contains(&key) {
        return Some(key - 256 + 223);
    }
    if key == KeyCode::MouseWheelUp as i32 {
        return Some(if family == KeyFamily::Q2 { 240 } else { 239 });
    }
    if key == KeyCode::MouseWheelDown as i32 {
        return Some(if family == KeyFamily::Q2 { 239 } else { 240 });
    }
    if family == KeyFamily::Q2 {
        let home = KeyCode::KeypadHome as i32;
        let plus = KeyCode::KeypadPlus as i32;
        if (home..=plus).contains(&key) {
            return Some(key);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        assert_eq!(string_to_keynum(None), -1);
        assert_eq!(string_to_keynum(Some("")), -1);
        assert_eq!(string_to_keynum(Some("a")), 97);
        assert_eq!(string_to_keynum(Some("SPACE")), KeyCode::Space as i32);
        assert_eq!(string_to_keynum(Some("space")), KeyCode::Space as i32);
        assert_eq!(string_to_keynum(Some("MWHEELUP")), KeyCode::MouseWheelUp as i32);
        assert_eq!(string_to_keynum(Some("JOY12")), KeyCode::Joy12 as i32);
        assert_eq!(string_to_keynum(Some("AUX17")), 256);
        assert_eq!(string_to_keynum(Some("KP_ENTER")), KeyCode::KeypadEnter as i32);
        assert_eq!(string_to_keynum(Some("0x41")), 65);
        assert_eq!(string_to_keynum(Some("0xzz")), 0);
        assert_eq!(string_to_keynum(Some("nope")), -1);
        assert_eq!(keynum_to_string(-1), "<KEY NOT FOUND>");
        assert_eq!(keynum_to_string(272), "<OUT OF RANGE>");
        assert_eq!(keynum_to_string(65), "A");
        assert_eq!(keynum_to_string(59), "SEMICOLON");
        assert_eq!(keynum_to_string(34), "0x22");
        assert_eq!(keynum_to_string(KeyCode::Space as i32), "SPACE");
        assert_eq!(keynum_to_string(0), "0x00");
    }

    #[test]
    fn source_numbers_follow_family_tables() {
        assert_eq!(source_key_number(65, KeyFamily::Q3), Some(65));
        assert_eq!(source_key_number(300, KeyFamily::Q3), None);
        assert_eq!(source_key_number(65, KeyFamily::Q1), Some(65));
        assert_eq!(
            source_key_number(KeyCode::Up as i32, KeyFamily::Q1),
            Some(KeyCode::Up as i32 - 4)
        );
        assert_eq!(
            source_key_number(KeyCode::Insert as i32, KeyFamily::Q2),
            Some(KeyCode::Insert as i32 + 8)
        );
        assert_eq!(
            source_key_number(KeyCode::F1 as i32, KeyFamily::Q1),
            Some(KeyCode::F1 as i32 - 10)
        );
        assert_eq!(source_key_number(KeyCode::Pause as i32, KeyFamily::Q1), Some(255));
        assert_eq!(
            source_key_number(KeyCode::Mouse1 as i32, KeyFamily::Q2),
            Some(200)
        );
        assert_eq!(
            source_key_number(KeyCode::MouseWheelUp as i32, KeyFamily::Q2),
            Some(240)
        );
        assert_eq!(
            source_key_number(KeyCode::MouseWheelUp as i32, KeyFamily::Q1),
            Some(239)
        );
        assert_eq!(
            source_key_number(KeyCode::KeypadHome as i32, KeyFamily::Q2),
            Some(KeyCode::KeypadHome as i32)
        );
        assert_eq!(source_key_number(KeyCode::KeypadHome as i32, KeyFamily::Q1), None);
        assert_eq!(source_key_number(256, KeyFamily::Q1), Some(223));
    }
}
