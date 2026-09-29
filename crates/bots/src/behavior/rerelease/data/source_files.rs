//! Bot source text from `src/bots/behavior/rerelease/data/source-files.ts`.
//!
//! The legacy text family uses byte characters; values above ASCII
//! pass through as Latin-1 without UTF-8 replacement, and trailing
//! NUL padding trims.

use crate::behavior::assets::BotSourceFiles;

/// Read source text for a path, or `None` when absent.
#[must_use]
pub fn read_bot_source_text(files: &dyn BotSourceFiles, path: &str) -> Option<String> {
    let bytes = files.read(path)?;
    let mut end = bytes.len();
    while end > 0 && bytes[end - 1] == 0 {
        end -= 1;
    }
    Some(bytes[..end].iter().map(|byte| *byte as char).collect())
}
