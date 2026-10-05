//! Source byte edit field (`cl_keys.c` fields).
//!
//! Donor provenance: `src/console/source-field.ts` (Quake III `cl_keys.c`,
//! id Software, GPL-2.0-or-later). Fixed 256-byte NUL-terminated buffer
//! with the donor's cursor/scroll/overstrike rules, paste recursion and
//! work budgets, and control-key bindings. Unicode console input stays in
//! [`super::field`].

use qa_client::input::KeyCode;

use super::ConsoleError;

/// Source field buffer size in bytes (255 text bytes plus NUL).
pub const FIELD_BUFFER_SIZE: usize = 256;
/// Maximum text bytes (excluding the terminator).
pub const FIELD_TEXT_SIZE: usize = 255;
/// Maximum nested clipboard reads per paste.
pub const FIELD_PASTE_DEPTH: u32 = 32;
/// Maximum byte operations per top-level paste.
pub const FIELD_PASTE_WORK: u32 = 65536;

/// Controls the host provides to the field (key state, overstrike, paste).
pub trait FieldControls {
    /// Whether a key code is currently held.
    fn is_down(&mut self, key: i32) -> bool;
    /// Current overstrike mode.
    fn get_overstrike(&mut self) -> bool;
    /// Set overstrike mode.
    fn set_overstrike(&mut self, value: bool);
    /// Read the clipboard, or [`None`] when unavailable (native Unix).
    fn clipboard_read(&mut self) -> Option<Vec<u8>>;
}

fn check_index(offset: usize) -> Result<(), ConsoleError> {
    if offset >= FIELD_BUFFER_SIZE {
        return Err(ConsoleError::BadField(
            "Undefined native field buffer index".to_string(),
        ));
    }
    Ok(())
}

/// Fixed 256-byte console edit field.
#[derive(Debug, Clone)]
pub struct EditField {
    buffer: [u8; FIELD_BUFFER_SIZE],
    paste_depth: u32,
    paste_work: u32,
    /// Cursor position in bytes.
    pub cursor: i32,
    /// Scroll offset in bytes.
    pub scroll: i32,
    /// Visible width in characters.
    pub width_in_chars: i32,
}

impl EditField {
    /// Open an empty field.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            buffer: [0; FIELD_BUFFER_SIZE],
            paste_depth: 0,
            paste_work: 0,
            cursor: 0,
            scroll: 0,
            width_in_chars: 0,
        }
    }

    /// Field text up to the first NUL byte.
    pub fn text(&self) -> Result<String, ConsoleError> {
        let mut text = String::new();
        for byte in self.buffer {
            if byte == 0 {
                return Ok(text);
            }
            text.push(byte as char);
        }
        Err(ConsoleError::BadField(
            "Undefined native unterminated field buffer".to_string(),
        ))
    }

    /// Read one raw buffer byte.
    pub fn read_byte(&self, offset: usize) -> Result<u8, ConsoleError> {
        check_index(offset)?;
        Ok(self.buffer[offset])
    }

    /// Write one raw buffer byte.
    pub fn write_byte(&mut self, offset: usize, byte: u8) -> Result<(), ConsoleError> {
        check_index(offset)?;
        self.buffer[offset] = byte;
        Ok(())
    }

    /// Clear the buffer, cursor, and scroll.
    pub fn clear(&mut self) {
        self.buffer = [0; FIELD_BUFFER_SIZE];
        self.cursor = 0;
        self.scroll = 0;
    }

    /// Replace the buffer contents (source bytes, at most 255).
    pub fn set_text(&mut self, value: &str) -> Result<(), ConsoleError> {
        let text = qa_core::cmd::source_command_text(value);
        if text.len() >= self.buffer.len() {
            return Err(ConsoleError::BadField("Field text exceeds 255 bytes".to_string()));
        }
        self.buffer = [0; FIELD_BUFFER_SIZE];
        for (index, byte) in text.bytes().enumerate() {
            self.buffer[index] = byte;
        }
        Ok(())
    }

    /// Copy another field's buffer, cursor, scroll, and width.
    pub fn copy_from(&mut self, source: &EditField) {
        self.buffer = source.buffer;
        self.cursor = source.cursor;
        self.scroll = source.scroll;
        self.width_in_chars = source.width_in_chars;
    }

    fn move_bytes(&mut self, destination: usize, start: usize, end: usize) -> Result<(), ConsoleError> {
        check_index(destination)?;
        check_index(start)?;
        if end < start || end > FIELD_BUFFER_SIZE || destination + end - start > FIELD_BUFFER_SIZE {
            return Err(ConsoleError::BadField(
                "Undefined native field memmove range".to_string(),
            ));
        }
        self.buffer.copy_within(start..end, destination);
        Ok(())
    }

    fn paste(&mut self, controls: &mut dyn FieldControls) -> Result<(), ConsoleError> {
        if self.paste_depth >= FIELD_PASTE_DEPTH {
            return Err(ConsoleError::BadField(
                "Recursive field paste exceeded 32 clipboard reads".to_string(),
            ));
        }
        let Some(bytes) = controls.clipboard_read() else {
            return Ok(());
        };
        if self.paste_depth == 0 {
            self.paste_work = 0;
        }
        self.paste_depth += 1;
        let mut result = Ok(());
        for byte in bytes {
            if byte == 0 {
                break;
            }
            self.paste_work += 1;
            if self.paste_work > FIELD_PASTE_WORK {
                result = Err(ConsoleError::BadField(
                    "Field paste exceeded 65536 byte operations".to_string(),
                ));
                break;
            }
            let character = if byte < 128 {
                i32::from(byte)
            } else {
                i32::from(byte) - 256
            };
            if let Err(error) = self.char_event(character, controls) {
                result = Err(error);
                break;
            }
        }
        self.paste_depth -= 1;
        result
    }

    /// Handle a key-down event.
    pub fn key_down(&mut self, key: i32, controls: &mut dyn FieldControls) -> Result<(), ConsoleError> {
        if (key == KeyCode::Insert as i32 || key == super::field::KEYPAD_INSERT)
            && controls.is_down(KeyCode::Shift as i32)
        {
            return self.paste(controls);
        }
        let length = self.text()?.len() as i32;
        if key == KeyCode::Delete as i32 {
            if self.cursor < length {
                self.move_bytes(self.cursor as usize, (self.cursor + 1) as usize, (length + 1) as usize)?;
            }
            return Ok(());
        }
        if key == KeyCode::Right as i32 {
            if self.cursor < length {
                self.cursor += 1;
            }
            if self.cursor >= self.scroll + self.width_in_chars && self.cursor <= length {
                self.scroll += 1;
            }
            return Ok(());
        }
        if key == KeyCode::Left as i32 {
            if self.cursor > 0 {
                self.cursor -= 1;
            }
            if self.cursor < self.scroll {
                self.scroll -= 1;
            }
            return Ok(());
        }
        if key == KeyCode::Home as i32 || ((key == 65 || key == 97) && controls.is_down(KeyCode::Control as i32)) {
            self.cursor = 0;
            return Ok(());
        }
        if key == KeyCode::End as i32 || ((key == 69 || key == 101) && controls.is_down(KeyCode::Control as i32)) {
            self.cursor = length;
            return Ok(());
        }
        if key == KeyCode::Insert as i32 {
            let overstrike = controls.get_overstrike();
            controls.set_overstrike(!overstrike);
        }
        Ok(())
    }

    /// Handle a character event (byte value, or 22/3/8/1/5 controls).
    pub fn char_event(&mut self, character: i32, controls: &mut dyn FieldControls) -> Result<(), ConsoleError> {
        if character == 22 {
            return self.paste(controls);
        }
        if character == 3 {
            self.clear();
            return Ok(());
        }
        let length = self.text()?.len() as i32;
        if character == 8 {
            if self.cursor > 0 {
                self.move_bytes((self.cursor - 1) as usize, self.cursor as usize, (length + 1) as usize)?;
                self.cursor -= 1;
                if self.cursor < self.scroll {
                    self.scroll -= 1;
                }
            }
            return Ok(());
        }
        if character == 1 {
            self.cursor = 0;
            self.scroll = 0;
            return Ok(());
        }
        if character == 5 {
            self.cursor = length;
            self.scroll = self.cursor - self.width_in_chars;
            return Ok(());
        }
        if character < 32 {
            return Ok(());
        }
        if character > 255 {
            return Err(ConsoleError::BadField(
                "Field characters require source bytes".to_string(),
            ));
        }
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let byte = character as u8;
        if controls.get_overstrike() {
            if self.cursor == FIELD_TEXT_SIZE as i32 {
                return Ok(());
            }
            check_index(self.cursor as usize)?;
            self.buffer[self.cursor as usize] = byte;
            self.cursor += 1;
        } else {
            if length == FIELD_TEXT_SIZE as i32 {
                return Ok(());
            }
            self.move_bytes((self.cursor + 1) as usize, self.cursor as usize, (length + 1) as usize)?;
            self.buffer[self.cursor as usize] = byte;
            self.cursor += 1;
        }
        if self.cursor >= self.width_in_chars {
            self.scroll += 1;
        }
        if self.cursor == length + 1 {
            check_index(self.cursor as usize)?;
            self.buffer[self.cursor as usize] = 0;
        }
        Ok(())
    }
}

impl Default for EditField {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    struct FixtureControls {
        down: HashSet<i32>,
        overstrike: bool,
        clipboard: Option<Vec<u8>>,
    }

    impl FieldControls for FixtureControls {
        fn is_down(&mut self, key: i32) -> bool {
            self.down.contains(&key)
        }

        fn get_overstrike(&mut self) -> bool {
            self.overstrike
        }

        fn set_overstrike(&mut self, value: bool) {
            self.overstrike = value;
        }

        fn clipboard_read(&mut self) -> Option<Vec<u8>> {
            self.clipboard.clone()
        }
    }

    fn controls() -> FixtureControls {
        FixtureControls {
            down: HashSet::new(),
            overstrike: false,
            clipboard: None,
        }
    }

    #[test]
    fn edits_bytes_with_overstrike() {
        let mut field = EditField::new();
        field.set_text("hello").unwrap();
        field.cursor = 5;
        let mut fixture = controls();
        field.char_event(i32::from(b'!'), &mut fixture).unwrap();
        assert_eq!(field.text().unwrap(), "hello!");
        fixture.overstrike = true;
        field.cursor = 0;
        field.char_event(i32::from(b'H'), &mut fixture).unwrap();
        assert_eq!(field.text().unwrap(), "Hello!");
        field.key_down(KeyCode::Delete as i32, &mut fixture).unwrap();
        assert_eq!(field.text().unwrap(), "Hllo!");
        field.key_down(KeyCode::Home as i32, &mut fixture).unwrap();
        assert_eq!(field.cursor, 0);
    }

    #[test]
    fn pastes_clipboard_and_rejects_overflow() {
        let mut field = EditField::new();
        let mut fixture = controls();
        fixture.clipboard = Some(b"ab\0cd".to_vec());
        field.char_event(22, &mut fixture).unwrap();
        assert_eq!(field.text().unwrap(), "ab");
        field.set_text(&"x".repeat(255)).unwrap();
        assert!(field.set_text(&"x".repeat(256)).is_err());
        assert!(field.char_event(256, &mut fixture).is_err());
        let mut copy = EditField::new();
        copy.copy_from(&field);
        assert_eq!(copy.text().unwrap(), "x".repeat(255));
    }

    #[test]
    fn control_keys_move_and_clear() {
        let mut field = EditField::new();
        field.set_text("hello").unwrap();
        field.cursor = 2;
        let mut fixture = controls();
        fixture.down.insert(KeyCode::Control as i32);
        field.key_down(69, &mut fixture).unwrap();
        assert_eq!(field.cursor, 5);
        field.char_event(3, &mut fixture).unwrap();
        assert_eq!(field.text().unwrap(), "");
    }
}
