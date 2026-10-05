//! `save` / `load` console command argument parsing.
//!
//! One function, no versions: `save <name>` and `load <name>` take no format
//! or product arguments. The engine picks the on-disk format internally from
//! the session classification (vanilla sessions write that game's legacy
//! format, everything else writes the proprietary format), and the loader
//! detects the format from the file contents.

use thiserror::Error;

/// Failure to parse a save/load request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SaveRequestError {
    /// Bad `save` arguments.
    #[error("Usage: save <name or path>")]
    BadSaveUsage,
    /// Bad `load` arguments.
    #[error("Usage: load <name or path>")]
    BadLoadUsage,
}

/// A parsed `save` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveRequest {
    /// Save name or path.
    pub name: String,
}

/// A parsed `load` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadRequest {
    /// Save name or path.
    pub name: String,
}

/// Parse `save <name or path>`.
pub fn parse_save_request(args: &[String]) -> Result<SaveRequest, SaveRequestError> {
    match args {
        [name] if !name.is_empty() => Ok(SaveRequest { name: name.clone() }),
        _ => Err(SaveRequestError::BadSaveUsage),
    }
}

/// Parse `load <name or path>`.
pub fn parse_load_request(args: &[String]) -> Result<LoadRequest, SaveRequestError> {
    match args {
        [name] if !name.is_empty() => Ok(LoadRequest { name: name.clone() }),
        _ => Err(SaveRequestError::BadLoadUsage),
    }
}

/// Static command documentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaveCommandDoc {
    /// One-line summary.
    pub summary: &'static str,
    /// Usage line.
    pub usage: &'static str,
    /// Examples.
    pub examples: &'static [&'static str],
}

/// Documentation for `save` / `load`; `None` for anything else.
#[must_use]
pub fn save_command_documentation(name: &str) -> Option<SaveCommandDoc> {
    match name {
        "save" => Some(SaveCommandDoc {
            summary: "Save the world in the format matching the session.",
            usage: "save <name or path>",
            examples: &["save quicksave"],
        }),
        "load" => Some(SaveCommandDoc {
            summary: "Restore a save; the format is detected from the file.",
            usage: "load <name or path>",
            examples: &["load quicksave"],
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn parses_save_form() {
        assert_eq!(
            parse_save_request(&args(&["quicksave"])).unwrap(),
            SaveRequest {
                name: "quicksave".to_string(),
            }
        );
    }

    #[test]
    fn rejects_bad_save_args() {
        for words in [
            &[][..],
            &[""][..],
            &["a", "v5"][..],
            &["a", "shared"][..],
            &["a", "b", "c"][..],
        ] {
            assert_eq!(
                parse_save_request(&args(words)).unwrap_err(),
                SaveRequestError::BadSaveUsage
            );
        }
    }

    #[test]
    fn parses_load_form() {
        assert_eq!(
            parse_load_request(&args(&["quicksave"])).unwrap(),
            LoadRequest {
                name: "quicksave".to_string(),
            }
        );
        for words in [&[][..], &[""][..], &["a", "b"][..], &["a", "b", "c"][..]] {
            assert_eq!(
                parse_load_request(&args(words)).unwrap_err(),
                SaveRequestError::BadLoadUsage
            );
        }
    }

    #[test]
    fn documents_save_commands() {
        let save = save_command_documentation("save").unwrap();
        assert!(!save.usage.is_empty());
        assert_eq!(save.examples.len(), 1);
        let load = save_command_documentation("load").unwrap();
        assert!(!load.usage.is_empty());
        assert!(save_command_documentation("delete").is_none());
    }
}
