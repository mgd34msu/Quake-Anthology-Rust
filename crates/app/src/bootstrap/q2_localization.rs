//! Quake II presentation-side localized text.
//!
//! Sync port of donor `src/app/bootstrap/q2-localization.ts`.
//! Presentation keeps unknown mod keys intact; the native table keeps its
//! byte-buffer contract. `qa_client::text::localization::LocalizationTable`
//! exists but keeps `find` private, so this module defines the minimal
//! catalog surface (`find` + `localize`) it needs; see absorbed-contracts.

/// One substitution slot in a localized format string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2LocArg {
    /// Argument index.
    pub arg_index: usize,
    /// Byte offset of the slot start in `format`.
    pub start: usize,
    /// Byte offset of the slot end in `format`.
    pub end: usize,
}

/// One parsed localization record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2LocEntry {
    /// Format string with `{...}` slots.
    pub format: String,
    /// Substitution slots in ascending offset order.
    pub arguments: Vec<Q2LocArg>,
}

/// Minimal catalog surface for Q2 presentation text.
pub trait Q2LocalizationCatalog {
    /// Look up a key without the leading `$`.
    fn find(&self, key: &str) -> Option<&Q2LocEntry>;
    /// Localize with the table default call (`allow_in_place`, 1024 bytes).
    fn localize(&self, text: &str, args: &[String]) -> String;
}

/// Resolve presentation text, preserving unknown `$keys` and trailing
/// newlines exactly like the donor.
#[must_use]
pub fn q2_localized_text(catalog: &(impl Q2LocalizationCatalog + ?Sized), text: &str, args: &[String]) -> String {
    let key = if text.starts_with('$') {
        text.trim_end_matches(['\r', '\n'])
    } else {
        text
    };
    let suffix = &text[key.len()..];
    let entry = if key.starts_with('$') {
        key.get(1..).and_then(|name| catalog.find(name))
    } else {
        None
    };
    if key.starts_with('$') && entry.is_none() {
        return text.to_string();
    }
    let Some(entry) = entry else {
        return if args.is_empty() {
            text.to_string()
        } else {
            catalog.localize(text, args)
        };
    };
    let mut result = String::new();
    let mut start = 0usize;
    for slot in &entry.arguments {
        let Some(argument) = args.get(slot.arg_index) else {
            return text.to_string();
        };
        let value = if argument.starts_with('$') {
            match argument.get(1..).and_then(|name| catalog.find(name)) {
                Some(_) => catalog.localize(argument, &[]),
                None => argument.clone(),
            }
        } else {
            argument.clone()
        };
        result.push_str(&entry.format[start..slot.start]);
        result.push_str(&value);
        start = slot.end;
    }
    result.push_str(&entry.format[start..]);
    result.push_str(suffix);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct FakeCatalog {
        entries: HashMap<String, Q2LocEntry>,
    }

    impl Q2LocalizationCatalog for FakeCatalog {
        fn find(&self, key: &str) -> Option<&Q2LocEntry> {
            self.entries.get(key)
        }

        fn localize(&self, text: &str, args: &[String]) -> String {
            if let Some(name) = text.strip_prefix('$') {
                if let Some(entry) = self.entries.get(name) {
                    let mut out = entry.format.clone();
                    for (index, arg) in args.iter().enumerate() {
                        out = out.replace(&format!("{{{index}}}"), arg);
                    }
                    return out;
                }
            }
            let mut out = text.to_string();
            for (index, arg) in args.iter().enumerate() {
                out = out.replace(&format!("{{{index}}}"), arg);
            }
            out
        }
    }

    fn catalog() -> FakeCatalog {
        FakeCatalog {
            entries: HashMap::from([
                (
                    "greeting".to_string(),
                    Q2LocEntry {
                        format: "Hello {0}!".to_string(),
                        arguments: vec![Q2LocArg {
                            arg_index: 0,
                            start: 6,
                            end: 9,
                        }],
                    },
                ),
                (
                    "name".to_string(),
                    Q2LocEntry {
                        format: "Quake".to_string(),
                        arguments: Vec::new(),
                    },
                ),
            ]),
        }
    }

    #[test]
    fn substitutes_and_keeps_suffix() {
        let catalog = catalog();
        assert_eq!(
            q2_localized_text(&catalog, "$greeting\r\n", &["Marine".to_string()]),
            "Hello Marine!\r\n"
        );
    }

    #[test]
    fn localizes_dollar_arguments() {
        let catalog = catalog();
        assert_eq!(
            q2_localized_text(&catalog, "$greeting", &["$name".to_string()]),
            "Hello Quake!"
        );
        assert_eq!(
            q2_localized_text(&catalog, "$greeting", &["$unknown".to_string()]),
            "Hello $unknown!"
        );
    }

    #[test]
    fn unknown_keys_and_missing_args_fall_back_to_text() {
        let catalog = catalog();
        assert_eq!(q2_localized_text(&catalog, "$unknown\r\n", &[]), "$unknown\r\n");
        assert_eq!(q2_localized_text(&catalog, "$greeting", &[]), "$greeting");
        assert_eq!(q2_localized_text(&catalog, "plain {0}", &["x".to_string()]), "plain x");
        assert_eq!(q2_localized_text(&catalog, "plain", &[]), "plain");
    }
}
