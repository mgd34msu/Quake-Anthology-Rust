//! Leased console history and bounded bitmap layout for the shared scene.
use crate::editor::Editor;
use qa_core::{
    events::TextStore,
    primitives::{RuleSetId, TextId, TextLease},
    sys_events::EventTime,
};

pub use qa_core::events::CONSOLE_DISPLAY_RECORDS as SCROLLBACK_RECORDS;

pub struct Scrollback {
    records: Box<[Option<TextLease>]>,
    first: usize,
    len: usize,
    pub evicted: u64,
    pub rejected: u64,
    revision: u64,
}
impl Default for Scrollback {
    fn default() -> Self {
        Self {
            records: (0..SCROLLBACK_RECORDS).map(|_| None).collect(),
            first: 0,
            len: 0,
            evicted: 0,
            rejected: 0,
            revision: 0,
        }
    }
}
impl Scrollback {
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn append(&mut self, texts: &mut TextStore, id: TextId) -> bool {
        let Some(lease) = texts.lease(id) else {
            self.rejected += 1;
            return false;
        };
        if self.len == self.records.len() {
            if let Some(old) = self.records[self.first].take() {
                texts.release(old);
            }
            self.first = (self.first + 1) % self.records.len();
            self.len -= 1;
            self.evicted += 1;
        }
        let slot = (self.first + self.len) % self.records.len();
        self.records[slot] = Some(lease);
        self.len += 1;
        self.revision = self.revision.wrapping_add(1);
        true
    }
    pub fn clear(&mut self, texts: &mut TextStore) {
        for record in &mut self.records {
            if let Some(lease) = record.take() {
                texts.release(lease);
            }
        }
        self.first = 0;
        self.len = 0;
        self.revision = self.revision.wrapping_add(1);
    }
    pub fn chunks<'a>(&'a self, texts: &'a TextStore) -> impl Iterator<Item = &'a [u8]> + Clone {
        (0..self.len).filter_map(move |index| {
            texts.get(
                self.records[(self.first + index) % self.records.len()]
                    .as_ref()?
                    .id(),
            )
        })
    }
}

#[derive(Clone, Copy)]
pub struct ConsoleView<'a> {
    pub editor: &'a Editor,
    pub scrollback: &'a Scrollback,
    pub texts: &'a TextStore,
    pub rules: RuleSetId,
    pub time: EventTime,
}

#[derive(Clone, Copy)]
pub struct Style {
    pub colors: bool,
    pub cursor_period_ms: u64,
    pub blink_cursor: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub glyph: u8,
    pub color: u8,
}
impl Default for Cell {
    fn default() -> Self {
        Self {
            glyph: b' ',
            color: 7,
        }
    }
}

/// Rebuilt display scratch contains glyph indices, never cloned event text.
pub struct Layout {
    cells: Box<[Cell]>,
    input: Box<[Cell]>,
    columns: usize,
    rows: usize,
    first: usize,
    used: usize,
    style: Style,
    revision: Option<u64>,
}
impl Layout {
    pub fn load(columns: usize, rows: usize, style: Style) -> Result<Self, &'static str> {
        let size = rows
            .checked_add(1)
            .and_then(|rows| columns.checked_mul(rows))
            .filter(|&size| size <= 65536)
            .ok_or("console layout capacity")?;
        if columns < 2 || rows == 0 || style.cursor_period_ms == 0 {
            return Err("invalid console layout");
        }
        Ok(Self {
            cells: vec![Cell::default(); size].into_boxed_slice(),
            input: vec![Cell::default(); columns].into_boxed_slice(),
            columns,
            rows,
            first: 0,
            used: 0,
            style,
            revision: None,
        })
    }
    pub fn columns(&self) -> usize {
        self.columns
    }
    pub fn rows(&self) -> usize {
        self.rows
    }
    pub fn prepare(&mut self, view: ConsoleView<'_>) {
        if self.revision != Some(view.scrollback.revision) {
            self.prepare_log(view);
            self.revision = Some(view.scrollback.revision);
        }
        self.input.fill(Cell::default());
        self.input[0].glyph = b']';
        let cursor = view.editor.line()[..view.editor.cursor()].chars().count();
        let start = (cursor + 1).saturating_sub(self.columns - 1);
        for (index, value) in view
            .editor
            .line()
            .chars()
            .skip(start)
            .take(self.columns - 1)
            .enumerate()
        {
            self.input[index + 1].glyph = u8::try_from(u32::from(value)).unwrap_or(b'?');
        }
        let phase = (view.time.milliseconds() / self.style.cursor_period_ms) & 1;
        if !self.style.blink_cursor || phase == 0 {
            let index = cursor - start + 1;
            self.input[index].glyph = 10
                + if self.style.blink_cursor {
                    0
                } else {
                    phase as u8
                };
        }
    }
    fn prepare_log(&mut self, view: ConsoleView<'_>) {
        self.prepare_bytes(view.scrollback.chunks(view.texts).flat_map(chunk_bytes));
    }
    /// HUD notification rows share console wrapping, colors and native glyph bits.
    /// Columns may shrink within the load-reserved row capacity for split views.
    pub fn prepare_text_chunks<'a>(
        &mut self,
        chunks: impl Iterator<Item = &'a [u8]> + Clone,
        columns: usize,
    ) -> Result<(), &'static str> {
        if columns < 2 || columns > self.input.len() {
            return Err("text columns exceed load capacity");
        }
        self.columns = columns;
        self.revision = None;
        self.prepare_bytes(chunks.flat_map(chunk_bytes));
        Ok(())
    }
    pub fn prepare_text(&mut self, text: &[u8], columns: usize) -> Result<(), &'static str> {
        self.prepare_text_chunks(std::iter::once(text), columns)
    }
    fn prepare_bytes(&mut self, bytes: impl Iterator<Item = (bool, u8)> + Clone) {
        self.cells.fill(Cell::default());
        let mut row = 0;
        let mut column = 0;
        let mut color = 7;
        let mut glyph_mask = 0;
        let mut word_start = true;
        let mut bytes = bytes.peekable();
        while let Some((first, byte)) = bytes.next() {
            if first && !self.style.colors {
                glyph_mask = if matches!(byte, 1 | 2) { 128 } else { 0 };
                if glyph_mask != 0 {
                    continue;
                }
            }
            if self.style.colors
                && byte == b'^'
                && let Some(next) = bytes
                    .peek()
                    .copied()
                    .map(|(_, next)| next)
                    .filter(|&next| next != b'^' && next != 0)
            {
                color = next.wrapping_sub(b'0') & 7;
                bytes.next();
                continue;
            }
            if byte == b'\r' {
                column = 0;
                word_start = true;
                continue;
            }
            if byte == b'\n' {
                self.next_row(&mut row, &mut column);
                word_start = true;
                continue;
            }
            if byte < b' ' {
                continue;
            }
            if word_start && byte != b' ' {
                let word = 1 + bytes
                    .clone()
                    .take_while(|&(_, byte)| byte > b' ')
                    .take(self.columns)
                    .count();
                if word < self.columns && column + word > self.columns {
                    self.next_row(&mut row, &mut column);
                }
            }
            word_start = byte == b' ';
            self.cells[(row % (self.rows + 1)) * self.columns + column] = Cell {
                glyph: byte | glyph_mask,
                color,
            };
            column += 1;
            if column == self.columns {
                self.next_row(&mut row, &mut column);
            }
        }
        let end = row + usize::from(column != 0);
        self.used = end.min(self.rows);
        self.first = end.saturating_sub(self.used) % (self.rows + 1);
    }
    fn next_row(&mut self, row: &mut usize, column: &mut usize) {
        *row += 1;
        *column = 0;
        let first = (*row % (self.rows + 1)) * self.columns;
        self.cells[first..first + self.columns].fill(Cell::default());
    }
    pub fn lines(&self) -> impl Iterator<Item = (usize, &[Cell])> {
        (0..self.used).map(|index| {
            let first = ((self.first + index) % (self.rows + 1)) * self.columns;
            (
                self.rows - self.used + index,
                &self.cells[first..first + self.columns],
            )
        })
    }
    pub fn input(&self) -> &[Cell] {
        &self.input
    }
}

fn chunk_bytes(bytes: &[u8]) -> impl Iterator<Item = (bool, u8)> + Clone + '_ {
    bytes
        .iter()
        .copied()
        .enumerate()
        .map(|(index, byte)| (index == 0, byte))
}
