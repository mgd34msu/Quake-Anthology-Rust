//! Bot script sources from `src/bots/behavior/library/script-sources.ts`
//! (`l_script.c`: `LoadScriptFile`, `PS_Token`).
//!
//! Loads text sources from the prepared asset files and serves
//! token streams to the character/chat/weight parsers. Includes
//! (`#include "file"`) resolve against the same prepared files with
//! cycle detection.

use std::collections::HashSet;

use super::structure::tokenize_bot_script;
use crate::behavior::assets::BotSourceFiles;
use crate::error::BotsError;

/// Maximum `#include` nesting depth.
pub const MAX_SCRIPT_INCLUDE_DEPTH: usize = 16;

/// One loaded script: path plus token stream.
#[derive(Debug, Clone)]
pub struct BotScript {
    /// Source path.
    pub path: String,
    /// Token stream.
    pub tokens: Vec<String>,
}

impl BotScript {
    /// Raw text reassembled from tokens (for diagnostics).
    #[must_use]
    pub fn token_count(&self) -> usize {
        self.tokens.len()
    }
}

/// Script source loader over prepared bot files.
pub struct BotScriptSources<'a> {
    files: &'a dyn BotSourceFiles,
    loaded: Vec<BotScript>,
}

impl<'a> BotScriptSources<'a> {
    /// New loader over prepared files.
    pub fn new(files: &'a dyn BotSourceFiles) -> Self {
        Self {
            files,
            loaded: Vec::new(),
        }
    }

    /// Load a script and its includes.
    pub fn load(&mut self, path: &str) -> Result<usize, BotsError> {
        let mut seen = HashSet::new();
        self.load_inner(path, 0, &mut seen)
    }

    fn load_inner(&mut self, path: &str, depth: usize, seen: &mut HashSet<String>) -> Result<usize, BotsError> {
        if depth > MAX_SCRIPT_INCLUDE_DEPTH {
            return Err(BotsError::BotScript(format!("script include depth exceeded at {path}")));
        }
        let key = path.to_lowercase();
        if !seen.insert(key) {
            return Err(BotsError::BotScript(format!("script include cycle at {path}")));
        }
        let bytes = self
            .files
            .read(path)
            .ok_or_else(|| BotsError::BotScript(format!("missing bot script {path}")))?;
        let text = String::from_utf8_lossy(&bytes);
        let mut tokens = Vec::new();
        for line in text.lines() {
            let trimmed = line.trim();
            if let Some(include) = trimmed
                .strip_prefix("#include")
                .map(str::trim)
                .and_then(|rest| rest.strip_prefix('"').and_then(|rest| rest.strip_suffix('"')))
            {
                let before = self.loaded.len();
                self.load_inner(include, depth + 1, seen)?;
                for script in self.loaded[before..].iter() {
                    tokens.extend(script.tokens.iter().cloned());
                }
                self.loaded.truncate(before);
            } else {
                tokens.extend(tokenize_bot_script(line));
            }
        }
        self.loaded.push(BotScript {
            path: path.to_owned(),
            tokens,
        });
        Ok(self.loaded.len() - 1)
    }

    /// Loaded scripts.
    #[must_use]
    pub fn scripts(&self) -> &[BotScript] {
        &self.loaded
    }

    /// Fetch a loaded script by index.
    #[must_use]
    pub fn script(&self, index: usize) -> Option<&BotScript> {
        self.loaded.get(index)
    }

    /// Clear loaded scripts.
    pub fn clear(&mut self) {
        self.loaded.clear();
    }
}

/// Read a `\0`-terminated C string from a byte reader.
#[must_use]
pub fn read_c_string(read_byte: &dyn Fn(usize) -> Option<u8>) -> String {
    let mut bytes = Vec::new();
    let mut index = 0;
    while let Some(byte) = read_byte(index) {
        if byte == 0 {
            break;
        }
        bytes.push(byte);
        index += 1;
    }
    String::from_utf8_lossy(&bytes).into_owned()
}
