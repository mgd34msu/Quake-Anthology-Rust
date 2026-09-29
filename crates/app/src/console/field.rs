//! Console edit field, history, and command completion.
//!
//! Donor provenance: `src/console/field.ts` (native menus and the rerelease
//! console edit Unicode by code point; source byte fields stay separate in
//! [`super::source_field`]). Key codes reuse
//! [`qa_client::input::KeyCode`]; the keypad codes below fill the keypad
//! range the client enum leaves out (donor `src/input/key-codes.ts`).

use qa_client::input::KeyCode;

/// Keypad Home (donor `KeyCode.KeypadHome`).
pub const KEYPAD_HOME: i32 = 160;
/// Keypad Up (donor `KeyCode.KeypadUp`).
pub const KEYPAD_UP: i32 = 161;
/// Keypad PageUp (donor `KeyCode.KeypadPageUp`).
pub const KEYPAD_PAGE_UP: i32 = 162;
/// Keypad Left (donor `KeyCode.KeypadLeft`).
pub const KEYPAD_LEFT: i32 = 163;
/// Keypad Right (donor `KeyCode.KeypadRight`).
pub const KEYPAD_RIGHT: i32 = 165;
/// Keypad End (donor `KeyCode.KeypadEnd`).
pub const KEYPAD_END: i32 = 166;
/// Keypad Down (donor `KeyCode.KeypadDown`).
pub const KEYPAD_DOWN: i32 = 167;
/// Keypad PageDown (donor `KeyCode.KeypadPageDown`).
pub const KEYPAD_PAGE_DOWN: i32 = 168;
/// Keypad Enter (donor `KeyCode.KeypadEnter`).
pub const KEYPAD_ENTER: i32 = 169;
/// Keypad Insert (donor `KeyCode.KeypadInsert`).
pub const KEYPAD_INSERT: i32 = 170;
/// Keypad Delete (donor `KeyCode.KeypadDelete`).
pub const KEYPAD_DELETE: i32 = 171;

/// Default edit-field capacity in code points.
pub const DEFAULT_FIELD_LENGTH: usize = 1023;
/// Default console history depth.
pub const DEFAULT_HISTORY_CAPACITY: usize = 32;

#[derive(Debug, Clone)]
struct Completion {
    text: String,
    cursor: usize,
    tail: String,
    matches: Vec<String>,
    index: isize,
    selected: String,
}

/// Unicode console edit field with completion cycling.
#[derive(Debug, Clone)]
pub struct ConsoleField {
    characters: Vec<char>,
    completion: Option<Completion>,
    /// Cursor position in code points.
    pub cursor: usize,
    /// Scroll offset in code points.
    pub scroll: usize,
    /// Overstrike mode.
    pub overstrike: bool,
    /// Maximum length in code points.
    pub maximum_length: usize,
    /// Visible width in characters.
    pub width_in_chars: usize,
}

impl ConsoleField {
    /// Open a field with the donor default length and width.
    #[must_use]
    pub fn new() -> Self {
        Self {
            characters: Vec::new(),
            completion: None,
            cursor: 0,
            scroll: 0,
            overstrike: false,
            maximum_length: DEFAULT_FIELD_LENGTH,
            width_in_chars: super::buffer::DEFAULT_WIDTH,
        }
    }

    /// Open a field with an explicit capacity and width.
    #[must_use]
    pub const fn with_capacity(maximum_length: usize, width_in_chars: usize) -> Self {
        Self {
            characters: Vec::new(),
            completion: None,
            cursor: 0,
            scroll: 0,
            overstrike: false,
            maximum_length,
            width_in_chars,
        }
    }

    /// Current field text.
    #[must_use]
    pub fn text(&self) -> String {
        self.characters.iter().collect()
    }

    /// Completion currently previewed, if the text still matches the cycle.
    #[must_use]
    pub fn selected_completion(&self) -> Option<&str> {
        self.completion.as_ref().and_then(|cycle| {
            if cycle.text == self.text() && cycle.cursor == self.cursor {
                Some(cycle.selected.as_str())
            } else {
                None
            }
        })
    }

    /// Cycle the leading command token through `names` (Tab completion).
    pub fn complete(&mut self, names: &[String], reverse: bool) {
        let mut cycle = self.completion.take();
        let same_cycle = cycle
            .as_ref()
            .is_some_and(|cycle| cycle.text == self.text() && cycle.cursor == self.cursor);
        if !same_cycle {
            let text = self.text();
            let (marker, token, tail) = split_leading_command(&text);
            let token_start = marker.chars().count();
            let token_end = token_start + token.chars().count();
            if self.cursor < token_start || self.cursor > token_end {
                return;
            }
            let prefix: String = token.chars().take(self.cursor.saturating_sub(token_start)).collect();
            let matches = completion_matches(&prefix, names);
            if matches.is_empty() {
                return;
            }
            cycle = Some(Completion {
                text,
                cursor: self.cursor,
                tail,
                matches,
                index: if reverse { 0 } else { -1 },
                selected: String::new(),
            });
        }
        let Some(mut cycle) = cycle else { return };
        let count = cycle.matches.len() as isize;
        cycle.index = (cycle.index + if reverse { -1 } else { 1 } + count) % count;
        #[allow(clippy::cast_sign_loss)]
        let selected = cycle.matches[cycle.index as usize].clone();
        let text = format!("/{selected}{}", cycle.tail);
        if text.chars().count() > self.maximum_length {
            self.completion = Some(cycle);
            return;
        }
        self.characters = text.chars().collect();
        self.cursor = 1 + selected.chars().count();
        self.keep_visible();
        cycle.text.clone_from(&text);
        cycle.cursor = self.cursor;
        cycle.selected = selected;
        self.completion = Some(cycle);
    }

    /// Replace the whole field (clears any completion cycle).
    pub fn set_text(&mut self, text: &str) {
        self.completion = None;
        self.characters = text
            .chars()
            .filter(|character| !matches!(*character, '\u{0}'..='\u{1f}' | '\u{7f}'))
            .take(self.maximum_length)
            .collect();
        self.cursor = self.characters.len();
        self.keep_visible();
    }

    /// Clear the field.
    pub fn clear(&mut self) {
        self.completion = None;
        self.characters.clear();
        self.cursor = 0;
        self.scroll = 0;
    }

    /// Insert text at the cursor, honoring overstrike mode.
    pub fn insert(&mut self, text: &str) {
        self.completion = None;
        for character in text.chars() {
            if character < ' ' || character == '\u{7f}' {
                continue;
            }
            if self.overstrike && self.cursor < self.characters.len() {
                self.characters[self.cursor] = character;
                self.cursor += 1;
            } else if self.characters.len() < self.maximum_length {
                self.characters.insert(self.cursor, character);
                self.cursor += 1;
            }
        }
        self.keep_visible();
    }

    /// Handle a key press; returns whether the field consumed it.
    pub fn key(
        &mut self,
        code: i32,
        control: bool,
        shift: bool,
        clipboard: &mut dyn FnMut() -> Option<String>,
    ) -> bool {
        if code != KeyCode::Shift as i32 && code != KeyCode::Control as i32 {
            self.completion = None;
        }
        if (code == 118 && control) || ((code == KeyCode::Insert as i32 || code == KEYPAD_INSERT) && shift) {
            if let Some(text) = clipboard() {
                let line = text.split(['\r', '\n', '\0']).next().unwrap_or("");
                self.insert(line);
            }
            return true;
        }
        if control {
            match code {
                97 => self.cursor = 0,
                101 => self.cursor = self.characters.len(),
                99 | 117 => self.clear(),
                107 => self.characters.truncate(self.cursor),
                119 => {
                    let start = self.word(-1);
                    self.characters.drain(start..self.cursor);
                    self.cursor = start;
                }
                _ => return self.navigation(code, true),
            }
            self.keep_visible();
            return true;
        }
        self.navigation(code, false)
    }

    fn word(&self, direction: i32) -> usize {
        let mut cursor = self.cursor as isize;
        let at = |cursor: isize| -> char {
            let index = if direction == -1 { cursor - 1 } else { cursor };
            if index < 0 {
                return '\0';
            }
            #[allow(clippy::cast_sign_loss)]
            self.characters.get(index as usize).copied().unwrap_or('\0')
        };
        let length = self.characters.len() as isize;
        while cursor + direction as isize >= 0 && cursor + direction as isize <= length && at(cursor).is_whitespace() {
            cursor += direction as isize;
        }
        while cursor + direction as isize >= 0
            && cursor + direction as isize <= length
            && at(cursor) != '\0'
            && !at(cursor).is_whitespace()
        {
            cursor += direction as isize;
        }
        cursor.max(0) as usize
    }

    fn navigation(&mut self, code: i32, control: bool) -> bool {
        match code {
            code if code == KeyCode::Backspace as i32 => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.characters.remove(self.cursor);
                }
            }
            code if code == KeyCode::Delete as i32 => {
                if self.cursor < self.characters.len() {
                    self.characters.remove(self.cursor);
                }
            }
            code if code == KeyCode::Left as i32 => {
                self.cursor = if control {
                    self.word(-1)
                } else {
                    self.cursor.saturating_sub(1)
                };
            }
            code if code == KeyCode::Right as i32 => {
                self.cursor = if control {
                    self.word(1)
                } else {
                    (self.cursor + 1).min(self.characters.len())
                };
            }
            code if code == KeyCode::Home as i32 => self.cursor = 0,
            code if code == KeyCode::End as i32 => self.cursor = self.characters.len(),
            code if code == KeyCode::Insert as i32 => self.overstrike = !self.overstrike,
            _ => return false,
        }
        self.keep_visible();
        true
    }

    fn keep_visible(&mut self) {
        self.scroll = self.scroll.clamp(0, self.cursor);
        if self.cursor >= self.scroll + self.width_in_chars {
            self.scroll = self.cursor - self.width_in_chars.max(1) + 1;
        }
    }
}

impl Default for ConsoleField {
    fn default() -> Self {
        Self::new()
    }
}

/// Split console input into an optional `/` or `\` marker, the leading
/// command token, and the tail (donor `/^([\\/]?)([^\\s]*)(.*)$/s`).
fn split_leading_command(text: &str) -> (String, String, String) {
    let mut chars = text.chars();
    let marker = match chars.clone().next() {
        Some('/' | '\\') => chars.next().unwrap_or('/').to_string(),
        _ => String::new(),
    };
    let rest: String = chars.collect();
    let token_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let token = rest[..token_end].to_string();
    let tail = rest[token_end..].to_string();
    (marker, token, tail)
}

fn fold_completion(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_uppercase() {
                character.to_ascii_lowercase()
            } else {
                character
            }
        })
        .collect()
}

/// Candidate names matching `prefix`, deduplicated by ASCII fold, sorted.
#[must_use]
pub fn completion_matches(prefix: &str, names: &[String]) -> Vec<String> {
    let mut unique: Vec<(String, String)> = Vec::new();
    for name in names {
        let folded = fold_completion(name);
        if folded.starts_with(&fold_completion(prefix)) && !unique.iter().any(|(folded_name, _)| folded_name == &folded)
        {
            unique.push((folded, name.clone()));
        }
    }
    unique.sort_by(|left, right| left.0.cmp(&right.0));
    unique.into_iter().map(|(_, name)| name).collect()
}

/// One-shot completion result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionResult {
    /// Completed text.
    pub text: String,
    /// All matches.
    pub matches: Vec<String>,
}

/// Complete the leading command token to its longest common prefix.
#[must_use]
pub fn complete_command(text: &str, names: &[String]) -> CompletionResult {
    let (marker, prefix, tail) = split_leading_command(text);
    if prefix.is_empty() {
        return CompletionResult {
            text: text.to_string(),
            matches: Vec::new(),
        };
    }
    let mut seen = std::collections::HashSet::new();
    let mut matches: Vec<String> = Vec::new();
    for name in names {
        if fold_completion(name).starts_with(&fold_completion(&prefix)) && seen.insert(name.clone()) {
            matches.push(name.clone());
        }
    }
    let Some(first) = matches.first().cloned() else {
        return CompletionResult {
            text: text.to_string(),
            matches,
        };
    };
    let mut common = first;
    for name in &matches {
        let mut length = 0;
        let common_chars: Vec<char> = common.chars().collect();
        let name_chars: Vec<char> = name.chars().collect();
        while length < common_chars.len()
            && length < name_chars.len()
            && fold_completion(&common_chars[length].to_string()) == fold_completion(&name_chars[length].to_string())
        {
            length += 1;
        }
        common = common_chars[..length].iter().collect();
    }
    let suffix = if tail.is_empty() && matches.len() == 1 {
        " ".to_string()
    } else {
        tail
    };
    CompletionResult {
        text: format!("{marker}{common}{suffix}"),
        matches,
    }
}

/// Bounded console history with a draft for mid-browse edits.
#[derive(Debug, Clone)]
pub struct ConsoleHistory {
    entries: Vec<String>,
    position: usize,
    draft: String,
    /// Maximum retained entries.
    pub capacity: usize,
}

impl ConsoleHistory {
    /// Open history with the donor default depth.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            position: 0,
            draft: String::new(),
            capacity: DEFAULT_HISTORY_CAPACITY,
        }
    }

    /// Open history with an explicit depth.
    #[must_use]
    pub const fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: Vec::new(),
            position: 0,
            draft: String::new(),
            capacity,
        }
    }

    /// Retained lines, oldest first.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        self.entries.clone()
    }

    /// Replace the history (restored settings), keeping the newest lines.
    pub fn replace(&mut self, lines: &[String]) {
        self.entries = lines
            .iter()
            .filter(|line| !line.is_empty())
            .cloned()
            .collect::<Vec<_>>();
        if self.entries.len() > self.capacity {
            let skip = self.entries.len() - self.capacity;
            self.entries.drain(..skip);
        }
        self.position = self.entries.len();
    }

    /// Append a submitted line (empty lines are ignored).
    pub fn add(&mut self, line: &str) {
        if line.is_empty() {
            return;
        }
        self.entries.push(line.to_string());
        if self.entries.len() > self.capacity {
            self.entries.remove(0);
        }
        self.position = self.entries.len();
        self.draft.clear();
    }

    /// Step toward older entries, stashing the current draft at the end.
    #[must_use]
    pub fn previous(&mut self, current: &str) -> String {
        if self.position == self.entries.len() {
            self.draft = current.to_string();
        }
        self.position = self.position.saturating_sub(1);
        self.entries
            .get(self.position)
            .cloned()
            .unwrap_or_else(|| current.to_string())
    }

    /// Step toward newer entries, restoring the draft past the end.
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> String {
        self.position = (self.position + 1).min(self.entries.len());
        self.entries
            .get(self.position)
            .cloned()
            .unwrap_or_else(|| self.draft.clone())
    }
}

impl Default for ConsoleHistory {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_insert_overstrike_and_words() {
        let mut field = ConsoleField::new();
        field.insert("hello world");
        assert_eq!(field.text(), "hello world");
        let mut clipboard = || None;
        field.key(KeyCode::Home as i32, false, false, &mut clipboard);
        assert_eq!(field.cursor, 0);
        field.key(KeyCode::Right as i32, true, false, &mut clipboard);
        assert_eq!(field.cursor, 5);
        field.overstrike = true;
        field.insert("X");
        assert_eq!(field.text(), "helloXworld");
        field.key(99, true, false, &mut clipboard);
        assert_eq!(field.text(), "");
    }

    #[test]
    fn pastes_first_line_and_kills_words() {
        let mut field = ConsoleField::new();
        let mut clipboard = || Some("pasted\nsecond".to_string());
        field.key(118, true, false, &mut clipboard);
        assert_eq!(field.text(), "pasted");
        field.key(119, true, false, &mut clipboard);
        assert_eq!(field.text(), "");
    }

    #[test]
    fn cycles_completions_with_slash_marker() {
        let mut field = ConsoleField::new();
        field.set_text("sta");
        let names = vec!["start".to_string(), "status".to_string(), "stop".to_string()];
        field.complete(&names, false);
        assert_eq!(field.text(), "/start");
        assert_eq!(field.selected_completion(), Some("start"));
        field.complete(&names, false);
        assert_eq!(field.text(), "/status");
        field.complete(&names, true);
        assert_eq!(field.text(), "/start");
    }

    #[test]
    fn completes_to_common_prefix() {
        let names = vec!["status".to_string(), "start".to_string()];
        let result = complete_command("st", &names);
        assert_eq!(result.text, "sta");
        assert_eq!(result.matches.len(), 2);
        let single = complete_command("stat", &names);
        assert_eq!(single.text, "status ");
        let empty = complete_command("", &names);
        assert!(empty.matches.is_empty());
    }

    #[test]
    fn history_browses_with_draft() {
        let mut history = ConsoleHistory::with_capacity(2);
        history.add("one");
        history.add("two");
        history.add("three");
        assert_eq!(history.lines(), vec!["two".to_string(), "three".to_string()]);
        assert_eq!(history.previous("draft"), "three");
        assert_eq!(history.previous("three"), "two");
        assert_eq!(history.next(), "three");
        assert_eq!(history.next(), "draft");
        history.replace(&["".to_string(), "kept".to_string()]);
        assert_eq!(history.lines(), vec!["kept".to_string()]);
    }
}
