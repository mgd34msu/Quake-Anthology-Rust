//! `save` / `load` console command argument parsing.
//!
//! Sync port of donor `src/app/bootstrap/save-requests.ts`.

use thiserror::Error;

/// Failure to parse a save/load request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SaveRequestError {
    /// Bad `save` arguments.
    #[error("Usage: save <name or path> [shared|v5|v6]")]
    BadSaveUsage,
    /// Bad `load` arguments.
    #[error("Usage: load <name or path> [source-product]")]
    BadLoadUsage,
}

/// Save file format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ApplicationSaveFormat {
    /// Shared cross-product format.
    #[default]
    Shared,
    /// Original NetQuake v5 format.
    V5,
    /// Original NetQuake v6 format.
    V6,
}

impl ApplicationSaveFormat {
    /// Donor wire spelling.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Shared => "shared",
            Self::V5 => "v5",
            Self::V6 => "v6",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "shared" => Some(Self::Shared),
            "v5" => Some(Self::V5),
            "v6" => Some(Self::V6),
            _ => None,
        }
    }
}

/// A parsed `save` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveRequest {
    /// Save name or path.
    pub name: String,
    /// Requested format.
    pub format: ApplicationSaveFormat,
}

/// A parsed `load` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadRequest {
    /// Save name or path.
    pub name: String,
    /// Source product disambiguating original Quake saves.
    pub source_product: Option<String>,
}

/// Parse `save <name or path> [shared|v5|v6]`.
pub fn parse_save_request(args: &[String]) -> Result<SaveRequest, SaveRequestError> {
    let name = args.first();
    let format = args.get(1).map(String::as_str).unwrap_or("shared");
    match name {
        Some(name) if !name.is_empty() && args.len() <= 2 => {
            match ApplicationSaveFormat::parse(format) {
                Some(format) => Ok(SaveRequest {
                    name: name.clone(),
                    format,
                }),
                None => Err(SaveRequestError::BadSaveUsage),
            }
        }
        _ => Err(SaveRequestError::BadSaveUsage),
    }
}

/// Parse `load <name or path> [source-product]`.
pub fn parse_load_request(args: &[String]) -> Result<LoadRequest, SaveRequestError> {
    let name = args.first();
    let source_product = args.get(1);
    if name.is_none_or(|name| name.is_empty())
        || args.len() > 2
        || source_product.is_some_and(|source| source.is_empty())
    {
        return Err(SaveRequestError::BadLoadUsage);
    }
    Ok(LoadRequest {
        name: name.cloned().unwrap_or_default(),
        source_product: source_product.cloned(),
    })
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
            summary: "Save the world; v5/v6 export the active singleplayer NetQuake source in its original format.",
            usage: "save <name or path> [shared|v5|v6]",
            examples: &["save quicksave", "save original v5"],
        }),
        "load" => Some(SaveCommandDoc {
            summary: "Restore a save. Specify the source product when an original Quake save has ambiguous content.",
            usage: "load <name or path> [source-product]",
            examples: &["load quicksave", "load original q1-classic-id1"],
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
    fn parses_save_forms() {
        assert_eq!(
            parse_save_request(&args(&["quicksave"])).unwrap(),
            SaveRequest {
                name: "quicksave".to_string(),
                format: ApplicationSaveFormat::Shared,
            }
        );
        assert_eq!(
            parse_save_request(&args(&["original", "v5"])).unwrap().format,
            ApplicationSaveFormat::V5
        );
        assert_eq!(
            parse_save_request(&args(&["original", "v6"])).unwrap().format,
            ApplicationSaveFormat::V6
        );
        assert_eq!(ApplicationSaveFormat::default(), ApplicationSaveFormat::Shared);
    }

    #[test]
    fn rejects_bad_save_args() {
        for words in [&[][..], &[""][..], &["a", "v5", "x"][..], &["a", "v7"][..]] {
            assert_eq!(
                parse_save_request(&args(words)).unwrap_err(),
                SaveRequestError::BadSaveUsage
            );
        }
        assert_eq!(
            SaveRequestError::BadSaveUsage.to_string(),
            "Usage: save <name or path> [shared|v5|v6]"
        );
    }

    #[test]
    fn parses_load_forms() {
        assert_eq!(
            parse_load_request(&args(&["quicksave"])).unwrap(),
            LoadRequest {
                name: "quicksave".to_string(),
                source_product: None,
            }
        );
        assert_eq!(
            parse_load_request(&args(&["original", "q1-classic-id1"])).unwrap().source_product,
            Some("q1-classic-id1".to_string())
        );
        for words in [&[][..], &[""][..], &["a", "b", "c"][..], &["a", ""][..]] {
            assert_eq!(
                parse_load_request(&args(words)).unwrap_err(),
                SaveRequestError::BadLoadUsage
            );
        }
    }

    #[test]
    fn documents_save_commands() {
        let save = save_command_documentation("save").unwrap();
        assert_eq!(save.usage, "save <name or path> [shared|v5|v6]");
        assert_eq!(save.examples.len(), 2);
        let load = save_command_documentation("load").unwrap();
        assert_eq!(load.usage, "load <name or path> [source-product]");
        assert!(save_command_documentation("delete").is_none());
    }
}
