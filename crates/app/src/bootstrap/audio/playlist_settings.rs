//! Playlist and menu-track settings.
//!
//! Port of donor `src/app/bootstrap/audio/playlist-settings.ts`
//! (`MusicPreferences`, `validMenuTrack`, `registerMusicSettings`,
//! `readMusicSettings`). The merged [`CvarRegistry`](qa_core::cvar::CvarRegistry)
//! has no binding, documentation, or alias surface, so validators,
//! documentation, and alias targets are ported as explicit module items with
//! the donor's exact texts; registration installs the two variables.

use qa_core::cvar::{flags, CvarError, CvarRegistry};
use qa_content::paths::normalize_resource_path;

use super::output_settings::is_js_trim;

/// Shuffle and menu-track preferences.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MusicPreferences {
    /// Shuffle mounted Quake II gameplay music at track end.
    pub music_shuffle: bool,
    /// Menu track selector (`auto`, `0`, a track number, or a path).
    pub menu_track: String,
}

/// Documentation for one music cvar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MusicCvarDoc {
    /// Summary text.
    pub summary: &'static str,
    /// Usage text.
    pub usage: &'static str,
    /// Example invocations.
    pub examples: &'static [&'static str],
}

/// `music_shuffle` documentation.
pub const MUSIC_SHUFFLE_DOC: MusicCvarDoc = MusicCvarDoc {
    summary: "Shuffle mounted Quake II gameplay music at track end. Menu music remains fixed.",
    usage: "music_shuffle <0|1>",
    examples: &["music_shuffle 1"],
};

/// `music_menu_track` documentation.
pub const MUSIC_MENU_TRACK_DOC: MusicCvarDoc = MusicCvarDoc {
    summary: "Menu music: auto uses Anthology's title/family fallback; 0 disables; numbers and paths select an explicit mounted track. Auto is an Anthology extension.",
    usage: "music_menu_track <auto|0|1..255|path>",
    examples: &["music_menu_track auto", "music_menu_track 77", "music_menu_track 0"],
};

/// Identity aliases for the music cvars: `(alias, target)`.
pub const MUSIC_SETTING_ALIASES: [(&str, &str); 2] = [
    ("ogg_shuffle", "music_shuffle"),
    ("ogg_menu_track", "music_menu_track"),
];

/// Whether a menu-track selector is well-formed.
#[must_use]
pub fn valid_menu_track(value: &str) -> bool {
    if value == "auto" || value == "0" {
        return true;
    }
    if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
        let number = value.parse::<f64>().unwrap_or(f64::NAN);
        return (1.0..=255.0).contains(&number);
    }
    if value.encode_utf16().count() > 255
        || value.trim_matches(is_js_trim) != value
        || value.bytes().any(|byte| {
            byte == b'"' || byte < 0x20 || byte == 0x7f || byte == b'\\'
        })
    {
        return false;
    }
    if normalize_resource_path(value).is_err() {
        return false;
    }
    let name = value.rsplit('/').next().unwrap_or("");
    if !name.contains('.') {
        return true;
    }
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".ogg") || lower.ends_with(".wav")
}

/// Validate a `music_shuffle` value (`None` accepts).
#[must_use]
pub fn validate_music_shuffle(text: &str) -> Option<&'static str> {
    if text == "0" || text == "1" {
        None
    } else {
        Some("Use 0 or 1")
    }
}

/// Validate a `music_menu_track` value (`None` accepts).
#[must_use]
pub fn validate_menu_track(text: &str) -> Option<&'static str> {
    if valid_menu_track(text) {
        None
    } else {
        Some("Use auto, 0, a track number 1..255, or a mounted OGG/WAV music path")
    }
}

/// Register the music settings cvars.
pub fn register_music_settings(cvars: &mut CvarRegistry) -> Result<(), CvarError> {
    cvars.register("music_shuffle", "0", flags::ARCHIVE)?;
    cvars.register("music_menu_track", "auto", flags::ARCHIVE)?;
    Ok(())
}

/// Read music settings, defaulting to unshuffled `auto` without a registry.
#[must_use]
pub fn read_music_settings(cvars: Option<&CvarRegistry>) -> MusicPreferences {
    let Some(cvars) = cvars else {
        return MusicPreferences { music_shuffle: false, menu_track: "auto".to_string() };
    };
    MusicPreferences {
        music_shuffle: cvars.variable_value("music_shuffle") != 0.0,
        menu_track: cvars.get("music_menu_track").map_or_else(|| "auto".to_string(), |found| found.value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::cmd::Dialect;

    #[test]
    fn accepts_selectors() {
        for value in ["auto", "0", "1", "007", "255", "music/track01", "music/win.ogg", "MUSIC/WIN.WAV", "2", "01x"] {
            assert!(valid_menu_track(value), "{value}");
        }
    }

    #[test]
    fn rejects_bad_selectors() {
        for value in ["", "256", "music/win.mp3", "music/win.", " music", "music ", "mus\"ic", "mus\\ic", "a/b/./c", "/abs", "music//x", "../x"] {
            assert!(!valid_menu_track(value), "{value:?}");
        }
        assert!(!valid_menu_track(&"a".repeat(256)));
        assert!(valid_menu_track(&format!("music/{}", "a".repeat(249))));
    }

    #[test]
    fn validates_cvar_text() {
        assert_eq!(validate_music_shuffle("0"), None);
        assert_eq!(validate_music_shuffle("1"), None);
        assert_eq!(validate_music_shuffle("2"), Some("Use 0 or 1"));
        assert_eq!(validate_music_shuffle(""), Some("Use 0 or 1"));
        assert_eq!(validate_menu_track("auto"), None);
        assert_eq!(
            validate_menu_track("music/win.mp3"),
            Some("Use auto, 0, a track number 1..255, or a mounted OGG/WAV music path")
        );
    }

    #[test]
    fn registers_and_reads_settings() {
        let mut cvars = CvarRegistry::new(Dialect::Q2Classic);
        register_music_settings(&mut cvars).unwrap();
        assert_eq!(cvars.get("music_shuffle").unwrap().value, "0");
        assert_eq!(cvars.get("music_menu_track").unwrap().value, "auto");
        let defaults = read_music_settings(Some(&cvars));
        assert_eq!(
            defaults,
            MusicPreferences { music_shuffle: false, menu_track: "auto".to_string() }
        );
        cvars.set("music_shuffle", "1", false).unwrap();
        cvars.set("music_menu_track", "77", false).unwrap();
        let updated = read_music_settings(Some(&cvars));
        assert_eq!(
            updated,
            MusicPreferences { music_shuffle: true, menu_track: "77".to_string() }
        );
        let missing = read_music_settings(None);
        assert_eq!(
            missing,
            MusicPreferences { music_shuffle: false, menu_track: "auto".to_string() }
        );
    }

    #[test]
    fn documents_aliases() {
        assert_eq!(
            MUSIC_SETTING_ALIASES,
            [("ogg_shuffle", "music_shuffle"), ("ogg_menu_track", "music_menu_track")]
        );
        assert_eq!(MUSIC_SHUFFLE_DOC.usage, "music_shuffle <0|1>");
        assert_eq!(MUSIC_MENU_TRACK_DOC.examples.len(), 3);
    }
}
