//! Scrollback buffer: word wrap, overwrite, colors, and notify times.
//!
//! Donor provenance: `src/console/buffer.ts` (Quake console output, id
//! Software, GPL-2.0-or-later). Same `^N` color escapes on Q3/Q2-rerelease,
//! `\x01`/`\x02` alternate prefix on other dialects, `[skipnotify]` prefix,
//! carriage-return overwrite, word-wrap lookahead, resize reflow around the
//! backscroll anchor, notify filtering, and `condump` text.

use qa_core::cmd::Dialect;

use super::ConsoleError;

/// One printed cell: glyph plus its color state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsoleCell {
    /// Printed glyph.
    pub character: char,
    /// Color index (`0..8`; donor default 7).
    pub color: u8,
    /// Set by the `\x01`/`\x02` line prefix on non-Q3 dialects.
    pub alternate: bool,
}

/// One immutable scrollback row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleRow {
    /// Monotonic row sequence (survives resize; restarts on clear).
    pub sequence: u64,
    /// Row cells (sparse: gaps after `\r` overwrite stay absent).
    pub cells: Vec<ConsoleCell>,
    /// Print time in milliseconds, or [`None`] for `[skipnotify]`/cleared.
    pub time_ms: Option<i64>,
}

#[derive(Debug, Clone)]
struct MutableRow {
    sequence: u64,
    wrapped: bool,
    cells: Vec<ConsoleCell>,
    time_ms: Option<i64>,
}

/// Resize reflow state: paragraphs rebuild into width-limited rows while
/// the backscroll anchor and write cursor track their offsets.
struct Reflow {
    rows: Vec<MutableRow>,
    paragraph: Vec<ConsoleCell>,
    anchor_offset: Option<usize>,
    cursor_offset: Option<usize>,
    new_anchor: usize,
    new_cursor: usize,
    new_write_row: usize,
    width: usize,
}

impl Reflow {
    fn new(width: usize) -> Self {
        Self {
            rows: Vec::new(),
            paragraph: Vec::new(),
            anchor_offset: None,
            cursor_offset: None,
            new_anchor: 0,
            new_cursor: 0,
            new_write_row: 0,
            width,
        }
    }

    fn push_row(&mut self, next_sequence: &mut u64, wrapped: bool, cells: Vec<ConsoleCell>) {
        let sequence = *next_sequence;
        *next_sequence += 1;
        self.rows.push(MutableRow {
            sequence,
            wrapped,
            cells,
            time_ms: None,
        });
    }

    fn flush(&mut self, next_sequence: &mut u64) {
        let start = self.rows.len();
        let mut offset = 0;
        loop {
            let end = (offset + self.width).min(self.paragraph.len());
            let wrapped = offset + self.width < self.paragraph.len();
            let cells = self.paragraph[offset..end].to_vec();
            self.push_row(next_sequence, wrapped, cells);
            if offset + self.width >= self.paragraph.len() {
                break;
            }
            offset += self.width;
        }
        if let Some(anchor) = self.anchor_offset {
            self.new_anchor = start + (self.rows.len() - start - 1).min(anchor / self.width);
        }
        if let Some(cursor) = self.cursor_offset {
            self.new_cursor = cursor % self.width;
            self.new_write_row = start + cursor / self.width;
            if self.new_write_row == self.rows.len() {
                if let Some(tail) = self.rows.last_mut() {
                    tail.wrapped = true;
                }
                self.push_row(next_sequence, false, Vec::new());
            }
        }
        self.paragraph.clear();
        self.anchor_offset = None;
        self.cursor_offset = None;
    }
}

/// Default console width in characters.
pub const DEFAULT_WIDTH: usize = 78;
/// Default scrollback capacity in characters.
pub const DEFAULT_CAPACITY: usize = 32768;
/// Default notify duration in milliseconds.
pub const DEFAULT_NOTIFY_MS: i64 = 3000;
/// Default notify row count.
pub const DEFAULT_NOTIFY_ROWS: usize = 4;

/// Word-wrapping console scrollback for one dialect.
#[derive(Debug, Clone)]
pub struct ConsoleBuffer {
    rows: Vec<MutableRow>,
    x: usize,
    write_row: usize,
    next_sequence: u64,
    backscroll: usize,
    dialect: Dialect,
    width: usize,
    character_capacity: usize,
}

impl ConsoleBuffer {
    /// Open a buffer with the donor default width and capacity.
    #[must_use]
    pub fn new(dialect: Dialect) -> Self {
        Self {
            rows: vec![MutableRow {
                sequence: 0,
                wrapped: false,
                cells: Vec::new(),
                time_ms: None,
            }],
            x: 0,
            write_row: 0,
            next_sequence: 1,
            backscroll: 0,
            dialect,
            width: DEFAULT_WIDTH,
            character_capacity: DEFAULT_CAPACITY,
        }
    }

    /// Open a buffer with an explicit width and character capacity.
    pub fn with_capacity(dialect: Dialect, width: usize, character_capacity: usize) -> Result<Self, ConsoleError> {
        if width < 1 || width > character_capacity || character_capacity == 0 {
            return Err(ConsoleError::BadWidth);
        }
        let mut buffer = Self::new(dialect);
        buffer.width = width;
        buffer.character_capacity = character_capacity;
        Ok(buffer)
    }

    /// Current print dialect.
    #[must_use]
    pub const fn dialect(&self) -> Dialect {
        self.dialect
    }

    /// Switch the print dialect (per-seat output mixes dialects).
    pub const fn set_dialect(&mut self, dialect: Dialect) {
        self.dialect = dialect;
    }

    /// Current width in characters.
    #[must_use]
    pub const fn width(&self) -> usize {
        self.width
    }

    /// Current backscroll depth in rows.
    #[must_use]
    pub const fn backscroll(&self) -> usize {
        self.backscroll
    }

    fn current(&mut self) -> Result<&mut MutableRow, ConsoleError> {
        self.rows.get_mut(self.write_row).ok_or(ConsoleError::NoCurrentRow)
    }

    fn linefeed(&mut self, time: Option<i64>, wrapped: bool) -> Result<(), ConsoleError> {
        let row = self.current()?;
        row.wrapped = wrapped;
        row.time_ms = time;
        self.write_row += 1;
        if self.write_row == self.rows.len() {
            if self.backscroll != 0 {
                self.backscroll += 1;
            }
            let sequence = self.next_sequence;
            self.next_sequence += 1;
            self.rows.push(MutableRow {
                sequence,
                wrapped: false,
                cells: Vec::new(),
                time_ms: time,
            });
        }
        self.x = 0;
        self.trim();
        Ok(())
    }

    fn trim(&mut self) {
        let maximum = (self.character_capacity / self.width).max(1);
        if self.rows.len() > maximum {
            let removed = self.rows.len() - maximum;
            self.rows.drain(..removed);
            self.write_row = self.write_row.saturating_sub(removed);
        }
        self.backscroll = self.backscroll.min(self.rows.len().saturating_sub(1));
    }

    fn color_escapes(&self) -> bool {
        matches!(self.dialect, Dialect::Q3 | Dialect::Q2Rerelease)
    }

    /// Print text at `time_ms`, wrapping and coloring per the dialect.
    pub fn print(&mut self, text: &str, time_ms: i64) -> Result<(), ConsoleError> {
        let (text, time) = if let Some(rest) = text.strip_prefix("[skipnotify]") {
            (rest, None)
        } else {
            (text, Some(time_ms))
        };
        let mut text = text;
        let mut alternate = false;
        if self.dialect != Dialect::Q3 && (text.starts_with('\x01') || text.starts_with('\x02')) {
            alternate = true;
            text = &text[1..];
        }
        let mut color: u8 = 7;
        let characters: Vec<char> = text.chars().collect();
        let mut index = 0;
        while index < characters.len() {
            let character = characters[index];
            let next = characters.get(index + 1).copied();
            if self.color_escapes() && character == '^' && next.is_some_and(|next| next != '^' && next.is_ascii_digit())
            {
                color = (next.map_or(0, |next| next as u8) - b'0') & 7;
                index += 2;
                continue;
            }
            if character == '\n' {
                self.linefeed(time, false)?;
                index += 1;
                continue;
            }
            if character == '\r' {
                self.x = 0;
                index += 1;
                continue;
            }
            if character > ' ' {
                let mut length = 0;
                while length < self.width && characters.get(index + length).is_some_and(|next| *next > ' ') {
                    length += 1;
                }
                if length < self.width && self.x + length >= self.width {
                    self.linefeed(time, true)?;
                }
            }
            let x = self.x;
            let row = self.current()?;
            if x < row.cells.len() {
                row.cells[x] = ConsoleCell {
                    character,
                    color,
                    alternate,
                };
            } else {
                row.cells.push(ConsoleCell {
                    character,
                    color,
                    alternate,
                });
            }
            row.time_ms = time;
            self.x += 1;
            if self.x >= self.width {
                self.linefeed(time, true)?;
            }
            index += 1;
        }
        Ok(())
    }

    /// Reflow the scrollback to a new width around the backscroll anchor.
    pub fn resize(&mut self, width: usize) -> Result<(), ConsoleError> {
        if width < 1 || width > self.character_capacity {
            return Err(ConsoleError::BadWidth);
        }
        if width == self.width {
            return Ok(());
        }
        let previous = std::mem::take(&mut self.rows);
        let anchor = previous.len().saturating_sub(1).saturating_sub(self.backscroll);
        let following = self.backscroll == 0;
        let mut reflow = Reflow::new(width);
        for (index, row) in previous.iter().enumerate() {
            if index == anchor {
                reflow.anchor_offset = Some(reflow.paragraph.len());
            }
            if index == self.write_row {
                reflow.cursor_offset = Some(reflow.paragraph.len() + self.x);
            }
            reflow.paragraph.extend_from_slice(&row.cells);
            if !row.wrapped {
                reflow.flush(&mut self.next_sequence);
            }
        }
        if !reflow.paragraph.is_empty() {
            reflow.flush(&mut self.next_sequence);
        }
        let rebuilt = reflow.rows;
        let (new_anchor, new_cursor, new_write_row) = (reflow.new_anchor, reflow.new_cursor, reflow.new_write_row);
        self.rows = rebuilt;
        self.width = width;
        self.x = new_cursor;
        self.write_row = new_write_row;
        self.backscroll = if following {
            0
        } else {
            self.rows.len().saturating_sub(1).saturating_sub(new_anchor)
        };
        self.trim();
        Ok(())
    }

    /// Drop all scrollback.
    pub fn clear(&mut self) {
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        self.rows = vec![MutableRow {
            sequence,
            wrapped: false,
            cells: Vec::new(),
            time_ms: None,
        }];
        self.x = 0;
        self.write_row = 0;
        self.backscroll = 0;
    }

    /// Clear notify times without dropping text.
    pub fn clear_notify(&mut self) {
        for row in &mut self.rows {
            row.time_ms = None;
        }
    }

    /// Scroll the backscroll anchor by `lines` (truncated toward zero).
    pub fn scroll(&mut self, lines: i64) {
        let lines = lines.clamp(-(self.rows.len() as i64), self.rows.len().saturating_sub(1) as i64);
        let backscroll = self.backscroll as i64 + lines;
        self.backscroll = backscroll.clamp(0, self.rows.len().saturating_sub(1) as i64).max(0) as usize;
    }

    /// Follow the newest output.
    pub const fn bottom(&mut self) {
        self.backscroll = 0;
    }

    /// Jump to the oldest output.
    pub fn top(&mut self) {
        self.backscroll = self.rows.len().saturating_sub(1);
    }

    /// Newest `count` rows above the backscroll anchor.
    #[must_use]
    pub fn visible(&self, count: usize) -> Vec<ConsoleRow> {
        let end = self.rows.len().saturating_sub(self.backscroll);
        let start = end.saturating_sub(count);
        self.rows[start..end]
            .iter()
            .map(|row| ConsoleRow {
                sequence: row.sequence,
                cells: row.cells.clone(),
                time_ms: row.time_ms,
            })
            .collect()
    }

    /// Rows printed within `duration_ms` of `now`, newest `count` at most.
    #[must_use]
    pub fn notifications(&self, now: i64, duration_ms: i64, count: usize) -> Vec<ConsoleRow> {
        let start = self.rows.len().saturating_sub(count);
        self.rows[start..]
            .iter()
            .filter(|row| row.time_ms.is_some_and(|time| now.wrapping_sub(time) <= duration_ms))
            .map(|row| ConsoleRow {
                sequence: row.sequence,
                cells: row.cells.clone(),
                time_ms: row.time_ms,
            })
            .collect()
    }

    /// Whole scrollback as `condump` text (trailing spaces trimmed per row).
    #[must_use]
    pub fn dump(&self) -> String {
        let mut text: String = self
            .rows
            .iter()
            .map(|row| {
                row.cells
                    .iter()
                    .map(|cell| cell.character)
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        text.push('\n');
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row_text(row: &ConsoleRow) -> String {
        row.cells.iter().map(|cell| cell.character).collect()
    }

    #[test]
    fn wraps_words_and_overwrites_returns() {
        let mut buffer = ConsoleBuffer::with_capacity(Dialect::Q3, 10, 1024).unwrap();
        buffer.print("hello world\n", 100).unwrap();
        buffer.print("overwrite\rBYE\n", 200).unwrap();
        assert_eq!(buffer.dump(), "hello\nworld\nBYErwrite\n\n");
    }

    #[test]
    fn colors_and_notify_follow_the_dialect() {
        let mut buffer = ConsoleBuffer::new(Dialect::Q3);
        buffer.print("^1red ^7plain\n", 1000).unwrap();
        buffer.print("[skipnotify]quiet\n", 1000).unwrap();
        let rows = buffer.visible(3);
        assert_eq!(rows[0].cells[0].color, 1);
        assert_eq!(rows[0].cells[4].color, 7);
        assert_eq!(rows[1].time_ms, None);
        assert_eq!(buffer.notifications(1000, DEFAULT_NOTIFY_MS, 4).len(), 1);
        assert_eq!(buffer.notifications(5000, DEFAULT_NOTIFY_MS, 4).len(), 0);

        let mut quake = ConsoleBuffer::new(Dialect::Q2Classic);
        quake.print("\x01center\n", 10).unwrap();
        assert!(quake.visible(1)[0].cells.iter().all(|cell| cell.alternate));
    }

    #[test]
    fn resize_reflows_around_the_anchor() {
        let mut buffer = ConsoleBuffer::with_capacity(Dialect::Q3, 8, 1024).unwrap();
        buffer.print("abcdefgh\n", 0).unwrap();
        buffer.print("ijklmnop\n", 0).unwrap();
        buffer.resize(4).unwrap();
        assert_eq!(buffer.width(), 4);
        assert_eq!(buffer.dump(), "abcd\nefgh\nijkl\nmnop\n\n");
        assert!(buffer.resize(0).is_err());
    }

    #[test]
    fn scroll_clamps_and_dumps() {
        let mut buffer = ConsoleBuffer::with_capacity(Dialect::Q2Classic, 78, 1024).unwrap();
        for line in 0..8 {
            buffer.print(&format!("line{line}\n"), i64::from(line)).unwrap();
        }
        buffer.scroll(2);
        assert_eq!(row_text(&buffer.visible(1)[0]), "line6");
        buffer.top();
        assert_eq!(row_text(&buffer.visible(1)[0]), "line0");
        buffer.bottom();
        assert_eq!(buffer.backscroll(), 0);
        buffer.clear_notify();
        assert!(buffer.notifications(100, DEFAULT_NOTIFY_MS, 8).is_empty());
        buffer.clear();
        assert_eq!(buffer.dump(), "\n");
    }
}
