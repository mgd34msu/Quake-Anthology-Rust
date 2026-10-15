//! One fixed input line for the shared console, fed only by queued input.
use qa_core::{sys_events::SeatId, text::FixedText};

pub const LINE_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditAction {
    Ignored,
    Caught,
    Submit,
}

#[derive(Default)]
pub struct Editor {
    seat: Option<SeatId>,
    line: FixedText<LINE_BYTES>,
    cursor: usize,
    overstrike: bool,
}

impl Editor {
    pub fn seat(&self) -> Option<SeatId> {
        self.seat
    }

    pub fn line(&self) -> &str {
        self.line.as_str()
    }

    /// Byte cursor is always a UTF-8 boundary in this line.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn toggle(&mut self, seat: SeatId) {
        self.seat = if self.seat == Some(seat) {
            None
        } else {
            Some(seat)
        };
    }

    pub fn set_active(&mut self, seat: SeatId, active: bool) {
        if active {
            self.seat = Some(seat);
        } else if self.seat == Some(seat) {
            self.seat = None;
        }
    }

    pub fn overstrike(&self) -> bool {
        self.overstrike
    }

    pub fn set_overstrike(&mut self, active: bool) {
        self.overstrike = active;
    }

    pub fn key(&mut self, seat: SeatId, control: u16, down: bool, repeat: bool) -> EditAction {
        // The neutral grave key is the native console toggle, independent of
        // bindings. Its repeat and key-up events never execute game commands.
        if control == 53 {
            if down && !repeat {
                self.toggle(seat);
            }
            return EditAction::Caught;
        }
        if self.seat != Some(seat) {
            return EditAction::Ignored;
        }
        if !down {
            return EditAction::Caught;
        }
        match control {
            40 | 88 if !repeat => return EditAction::Submit,
            41 if !repeat => self.seat = None,
            73 if !repeat => self.overstrike = !self.overstrike,
            42 => {
                let start = self.adjacent(false);
                let _ = self.line.replace(start..self.cursor, "");
                self.cursor = start;
            }
            76 => {
                let end = self.adjacent(true);
                let _ = self.line.replace(self.cursor..end, "");
            }
            74 => self.cursor = 0,
            77 => self.cursor = self.line.as_bytes().len(),
            79 => self.cursor = self.adjacent(true),
            80 => self.cursor = self.adjacent(false),
            _ => {}
        }
        EditAction::Caught
    }

    /// Text is inserted atomically; full lines retain both content and cursor.
    pub fn character(&mut self, seat: SeatId, value: char) -> std::fmt::Result {
        if self.seat != Some(seat) || value.is_control() || matches!(value, '`' | '~') {
            return Ok(());
        }
        let mut bytes = [0; 4];
        let text = value.encode_utf8(&mut bytes);
        let end = if self.overstrike {
            self.adjacent(true)
        } else {
            self.cursor
        };
        self.line.replace(self.cursor..end, text)?;
        self.cursor += text.len();
        Ok(())
    }

    /// Called only after admission into the sole console command buffer.
    pub fn submitted(&mut self) {
        self.line.clear();
        self.cursor = 0;
    }

    fn adjacent(&self, forward: bool) -> usize {
        if forward {
            self.line.as_str()[self.cursor..]
                .chars()
                .next()
                .map_or(self.cursor, |value| self.cursor + value.len_utf8())
        } else {
            self.line.as_str()[..self.cursor]
                .char_indices()
                .next_back()
                .map_or(0, |(index, _)| index)
        }
    }
}
