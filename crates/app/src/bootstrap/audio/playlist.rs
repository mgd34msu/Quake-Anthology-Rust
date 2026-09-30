//! Mounted music discovery, cue quoting, and shuffle.
//!
//! Port of donor `src/app/bootstrap/audio/playlist.ts`
//! (`mountedMusicTracks`, `musicFileCue`, `shuffledTracks`). The donor's
//! async mount calls run synchronously here; ordering matches JavaScript
//! (`sort` compares UTF-16 units).

use std::collections::HashSet;

use qa_content::mounts::{MountError, MountedContent};

use super::playlist_settings::valid_menu_track;

/// Directory listing surface used for music discovery.
pub trait MusicMounts {
    /// List names under a directory with an extension filter.
    fn list_files(&self, directory: &str, extension: &str) -> Result<Vec<String>, MountError>;
}

impl MusicMounts for MountedContent {
    fn list_files(&self, directory: &str, extension: &str) -> Result<Vec<String>, MountError> {
        MountedContent::list_files(self, directory, extension)
    }
}

/// List supported music under `music/`, including nested directories.
pub fn mounted_music_tracks(mounts: &impl MusicMounts) -> Result<Vec<String>, MountError> {
    let mut tracks = HashSet::new();
    let mut directories = vec!["music".to_string()];
    let mut seen = HashSet::new();
    let mut index = 0;
    while index < directories.len() && index < 4095 && tracks.len() < 4095 {
        let directory = directories[index].clone();
        index += 1;
        if directory.encode_utf16().count() >= 256 || !seen.insert(directory.clone()) {
            continue;
        }
        for extension in [".ogg", ".wav"] {
            for name in mounts.list_files(&directory, extension)? {
                if tracks.len() == 4095 {
                    break;
                }
                let path = format!("{directory}/{name}");
                if valid_menu_track(&path) {
                    tracks.insert(path);
                }
            }
        }
        if directory.split('/').count() < 16 {
            for name in mounts.list_files(&directory, "/")? {
                if directories.len() == 4095 {
                    break;
                }
                let child = name.trim_end_matches(['/', '\\']);
                if !child.is_empty() {
                    directories.push(format!("{directory}/{child}"));
                }
            }
        }
    }
    let mut tracks: Vec<String> = tracks.into_iter().collect();
    tracks.sort_by(|left, right| left.encode_utf16().cmp(right.encode_utf16()));
    Ok(tracks)
}

/// JavaScript `\s` character set.
pub(crate) fn is_js_space(char: char) -> bool {
    matches!(
        char,
        '\u{9}'
            | '\u{a}'
            | '\u{b}'
            | '\u{c}'
            | '\u{d}'
            | '\u{20}'
            | '\u{a0}'
            | '\u{1680}'
            | '\u{2000}'
            | '\u{2001}'
            | '\u{2002}'
            | '\u{2003}'
            | '\u{2004}'
            | '\u{2005}'
            | '\u{2006}'
            | '\u{2007}'
            | '\u{2008}'
            | '\u{2009}'
            | '\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
            | '\u{feff}'
    )
}

/// Quote a cue path containing JavaScript whitespace.
#[must_use]
pub fn music_file_cue(path: &str) -> String {
    if path.chars().any(is_js_space) {
        format!("\"{path}\"")
    } else {
        path.to_string()
    }
}

/// Shuffle tracks without repeating the previous cue first.
pub fn shuffled_tracks(tracks: &[String], previous: &str, random: &mut dyn FnMut() -> f64) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut bag: Vec<String> = tracks
        .iter()
        .filter(|track| seen.insert((*track).clone()))
        .cloned()
        .collect();
    let mut index = bag.len();
    while index > 1 {
        index -= 1;
        let selected = (random() * (index + 1) as f64).floor();
        if selected >= 0.0 && selected <= index as f64 {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            bag.swap(index, selected as usize);
        }
    }
    if bag.len() > 1 && bag.first().is_some_and(|first| music_file_cue(first) == previous) {
        let first = bag.remove(0);
        bag.push(first);
    }
    bag
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeMounts {
        files: Vec<(String, String, Vec<String>)>,
    }

    impl MusicMounts for FakeMounts {
        fn list_files(&self, directory: &str, extension: &str) -> Result<Vec<String>, MountError> {
            Ok(self
                .files
                .iter()
                .find(|(dir, ext, _)| dir == directory && ext == extension)
                .map_or_else(Vec::new, |(_, _, names)| names.clone()))
        }
    }

    #[test]
    fn discovers_nested_tracks_sorted() {
        let mounts = FakeMounts {
            files: vec![
                ("music".to_string(), ".ogg".to_string(), vec!["b.ogg".to_string()]),
                (
                    "music".to_string(),
                    ".wav".to_string(),
                    vec!["a.wav".to_string(), "skip.mp3".to_string()],
                ),
                ("music".to_string(), "/".to_string(), vec!["sub/".to_string()]),
                ("music/sub".to_string(), ".ogg".to_string(), vec!["c.ogg".to_string()]),
                ("music/sub".to_string(), ".wav".to_string(), vec![]),
                ("music/sub".to_string(), "/".to_string(), vec![]),
            ],
        };
        assert_eq!(
            mounted_music_tracks(&mounts).unwrap(),
            ["music/a.wav", "music/b.ogg", "music/sub/c.ogg"]
        );
    }

    #[test]
    fn quotes_whitespace_cues() {
        assert_eq!(music_file_cue("music/win"), "music/win");
        assert_eq!(music_file_cue("music/my win"), "\"music/my win\"");
        assert_eq!(music_file_cue("music/a\tb"), "\"music/a\tb\"");
    }

    #[test]
    fn shuffles_without_leading_repeat() {
        let tracks = ["a".to_string(), "b".to_string(), "c".to_string()];
        let mut calls = 0;
        let mut random = || {
            calls += 1;
            0.0
        };
        let bag = shuffled_tracks(&tracks, "", &mut random);
        assert_eq!(bag.len(), 3);
        assert_eq!(calls, 2);
        // Deterministic zero shuffle rotates [a, b, c] to [b, c, a].
        assert_eq!(bag, ["b", "c", "a"]);
        // A previous cue matching the head rotates the head to the tail.
        let bag = shuffled_tracks(&tracks, "b", &mut || 0.0);
        assert_eq!(bag, ["c", "a", "b"]);
        // Duplicates collapse.
        let bag = shuffled_tracks(&["a".to_string(), "a".to_string()], "", &mut || 0.0);
        assert_eq!(bag, ["a"]);
    }
}
