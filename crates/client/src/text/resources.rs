//! Localization resource loading (`loc_*.txt` overlays).
//!
//! Donor provenance: `src/text/localization-resources.ts`. English fills
//! missing entries; the selected language and its mod overlay take
//! precedence. Sync port of the donor's async reads.

use qa_core::identity::SeatId;

use super::localization::{LocReloadOptions, LocalizationCatalog, LocalizationProfile, LocalizationTable};

/// A localization reader.
pub trait LocalizationReader {
    /// Read file bytes.
    fn read(&mut self, path: &str) -> Option<Vec<u8>>;
}

fn load_resources(catalog: &mut LocalizationTable, language: &str, read: &mut dyn LocalizationReader) {
    let names: Vec<&str> = if language == "english" {
        vec!["english"]
    } else {
        vec!["english", language]
    };
    for name in names {
        let base = read.read(&format!("localization/loc_{name}.txt"));
        if let Some(base) = base {
            catalog.merge(Some(&base), &LocReloadOptions::default(), None);
        }
        let overlay = read.read(&format!("localization/loc_{name}_mod.txt"));
        if let Some(overlay) = overlay {
            catalog.merge(Some(&overlay), &LocReloadOptions::default(), None);
        }
    }
}

/// Load seat resources (`loadLocalizationResources`).
#[must_use]
pub fn load_localization_resources(
    seat: SeatId,
    language: &str,
    read: &mut dyn LocalizationReader,
    profile: LocalizationProfile,
) -> LocalizationCatalog {
    let mut catalog = LocalizationCatalog::new(seat, profile);
    load_resources(&mut catalog.table, language, read);
    catalog
}

/// Load server resources without a player seat
/// (`loadServerLocalizationResources`).
#[must_use]
pub fn load_server_localization_resources(
    language: &str,
    read: &mut dyn LocalizationReader,
    profile: LocalizationProfile,
) -> LocalizationTable {
    let mut table = LocalizationTable::new(profile);
    load_resources(&mut table, language, read);
    table
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    struct Fixed {
        files: Vec<(String, Vec<u8>)>,
    }

    impl LocalizationReader for Fixed {
        fn read(&mut self, path: &str) -> Option<Vec<u8>> {
            self.files
                .iter()
                .find(|(name, _)| name == path)
                .map(|(_, bytes)| bytes.clone())
        }
    }

    #[test]
    fn english_fills_missing_entries() {
        let owner = IdentityOwner::create("test").unwrap();
        let mut read = Fixed {
            files: vec![(
                "localization/loc_english.txt".to_string(),
                b"HELLO = \"Hello\"\n".to_vec(),
            )],
        };
        let catalog = load_localization_resources(owner.seat(0), "french", &mut read, LocalizationProfile::Q1Rerelease);
        assert_eq!(catalog.localize("$HELLO", &[]), "Hello");
    }

    #[test]
    fn language_overrides_english() {
        let owner = IdentityOwner::create("test").unwrap();
        let mut read = Fixed {
            files: vec![
                (
                    "localization/loc_english.txt".to_string(),
                    b"HELLO = \"Hello\"\n".to_vec(),
                ),
                (
                    "localization/loc_french.txt".to_string(),
                    b"HELLO = \"Bonjour\"\n".to_vec(),
                ),
            ],
        };
        let catalog = load_localization_resources(owner.seat(0), "french", &mut read, LocalizationProfile::Q1Rerelease);
        assert_eq!(catalog.localize("$HELLO", &[]), "Bonjour");
    }
}
