//! Content directory inputs and executable-relative discovery.
//!
//! The donor defaults its game data root to a home-directory path (see
//! `src/app/bootstrap/options.ts`). Released builds cannot assume any home
//! layout, so resolution here is explicit user input first, then
//! executable-relative discovery, then the executable's own directory.
//! Discovery checks the executable's own directory and then its parent for
//! engine game-data markers, so dropping the executable either directly
//! beside the installed games or in one folder below them resolves without
//! configuration. No install-folder names are assumed; only id Software's
//! canonical game-data directory names act as markers.

use std::path::{Path, PathBuf};

/// Canonical game-data directory names recognized by discovery.
///
/// These are engine constants, not install layouts: id Software's `id1`,
/// the Quake mission-pack directories, QuakeWorld's `qw`, and the Quake II
/// / III base directories from the donor catalog (`content/catalog/*`).
pub const GAME_DATA_MARKERS: &[&str] = &["id1", "hipnotic", "rogue", "qw", "baseq2", "baseq3", "missionpack"];

/// Archive suffixes that mark a directory as content-bearing when a matching
/// file sits directly inside it.
pub const ARCHIVE_SUFFIXES: &[&str] = &["pak", "pk3", "wad"];

/// How many directory levels below a candidate discovery descends looking
/// for a marker. Three covers an install folder nested under the candidate
/// as well as the donor's normalized `q2/rerelease/baseq2` nesting.
pub const DISCOVERY_DEPTH: usize = 3;

/// One directory entry observed by a [`DirProbe`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirChild {
    /// Entry file name.
    pub name: String,
    /// True for directories (symlinks excluded, so traversal cannot cycle).
    pub is_dir: bool,
}

/// Filesystem access for discovery, injectable so tests run headless.
pub trait DirProbe {
    /// List the direct children of `dir`, or an empty list when `dir` is
    /// missing, unreadable, or not a directory.
    fn children(&self, dir: &Path) -> Vec<DirChild>;
}

/// [`DirProbe`] over the real filesystem.
#[derive(Debug, Clone, Copy, Default)]
pub struct FsProbe;

impl DirProbe for FsProbe {
    fn children(&self, dir: &Path) -> Vec<DirChild> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        entries
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let file_type = entry.file_type().ok()?;
                Some(DirChild {
                    name: entry.file_name().to_string_lossy().into_owned(),
                    is_dir: file_type.is_dir(),
                })
            })
            .collect()
    }
}

/// True when `name` (already lowercased) is a content archive file name.
fn is_archive_name(name: &str) -> bool {
    name.rsplit('.')
        .next()
        .is_some_and(|ext| ARCHIVE_SUFFIXES.contains(&ext))
}

/// True when `dir` looks like a game content root: a marker directory at
/// most [`DISCOVERY_DEPTH`] levels below it, or a content archive directly
/// inside it. Name matching is ASCII case-insensitive so installs like
/// `ID1` or `BaseQ2` resolve on any host.
#[must_use]
pub fn looks_like_content_dir(dir: &Path, probe: &impl DirProbe) -> bool {
    let mut frontier = vec![dir.to_path_buf()];
    for depth in 1..=DISCOVERY_DEPTH {
        let mut next = Vec::new();
        for current in &frontier {
            for child in probe.children(current) {
                let lower = child.name.to_ascii_lowercase();
                if child.is_dir {
                    if GAME_DATA_MARKERS.contains(&lower.as_str()) {
                        return true;
                    }
                    next.push(current.join(&child.name));
                } else if depth == 1 && is_archive_name(&lower) {
                    return true;
                }
            }
        }
        if next.is_empty() {
            return false;
        }
        frontier = next;
    }
    false
}

/// Find the game content root relative to the executable directory: the
/// executable's own directory first (dropped beside the installs), then its
/// parent (dropped in one folder below them).
#[must_use]
pub fn discover_corpus_root(exe_dir: &Path, probe: &impl DirProbe) -> Option<PathBuf> {
    if looks_like_content_dir(exe_dir, probe) {
        return Some(exe_dir.to_path_buf());
    }
    if let Some(parent) = exe_dir.parent() {
        if looks_like_content_dir(parent, probe) {
            return Some(parent.to_path_buf());
        }
    }
    None
}

/// Resolve the game data root: explicit user input first, then
/// executable-relative discovery, then the executable's own directory (the
/// portable layout: content lives next to the binary), then the working
/// directory when the executable path is unknown. Blank explicit input
/// counts as unset.
#[must_use]
pub fn resolve_corpus_root(explicit: Option<&str>, exe_dir: Option<&Path>, probe: &impl DirProbe) -> String {
    if let Some(path) = explicit.map(str::trim).filter(|path| !path.is_empty()) {
        return path.to_string();
    }
    if let Some(dir) = exe_dir {
        if let Some(found) = discover_corpus_root(dir, probe) {
            return found.to_string_lossy().into_owned();
        }
        return dir.to_string_lossy().into_owned();
    }
    std::env::current_dir().map_or_else(|_| ".".to_string(), |cwd| cwd.to_string_lossy().into_owned())
}

/// Per-item directory inputs. Each field is an explicit user override;
/// [`None`] selects the automatic default for that item.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContentDirectories {
    /// Game data root override.
    pub corpus_root: Option<String>,
    /// Writable user content root override.
    pub user_content_root: Option<String>,
}

impl ContentDirectories {
    /// Resolve the game data root (explicit input, discovery, exe dir).
    #[must_use]
    pub fn corpus_root(&self, exe_dir: Option<&Path>, probe: &impl DirProbe) -> String {
        resolve_corpus_root(self.corpus_root.as_deref(), exe_dir, probe)
    }

    /// Resolve the writable user content root: explicit input or `default`.
    /// Discovery never applies here; the executable directory may be
    /// read-only, and discovery exists to find games, not to place writes.
    #[must_use]
    pub fn user_content_root(&self, default: String) -> String {
        if let Some(path) = self
            .user_content_root
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
        {
            return path.to_string();
        }
        default
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Fake filesystem: every known directory maps to its children.
    #[derive(Debug, Default)]
    struct FakeFs {
        dirs: HashMap<PathBuf, Vec<DirChild>>,
    }

    impl FakeFs {
        fn with(mut self, dir: &str, children: &[(&str, bool)]) -> Self {
            self.dirs.insert(
                PathBuf::from(dir),
                children
                    .iter()
                    .map(|(name, is_dir)| DirChild {
                        name: (*name).to_string(),
                        is_dir: *is_dir,
                    })
                    .collect(),
            );
            self
        }
    }

    impl DirProbe for FakeFs {
        fn children(&self, dir: &Path) -> Vec<DirChild> {
            self.dirs.get(dir).cloned().unwrap_or_default()
        }
    }

    fn dir(name: &str) -> (&str, bool) {
        (name, true)
    }

    fn file(name: &str) -> (&str, bool) {
        (name, false)
    }

    /// Every root/install/data-dir combination must resolve identically:
    /// discovery keys on engine data directories, never on install or root
    /// names, so no name in this battery is special.
    #[test]
    fn install_and_root_names_are_not_assumed() {
        let roots = ["/home/operator/retro", "/mnt/archive/fps", "/srv/media/old"];
        let installs = [
            "Quake",
            "quake",
            "Quake II",
            "QUAKE-III-ARENA",
            "q2",
            "third install",
            "x",
        ];
        let data_dirs = ["id1", "hipnotic", "rogue", "qw", "baseq2", "baseq3", "missionpack"];
        for root in roots {
            for install in installs {
                for data in data_dirs {
                    let install_dir = format!("{root}/{install}");
                    let fs = FakeFs::default()
                        .with(root, &[dir(install), file("qa-muse")])
                        .with(&install_dir, &[dir(data)]);
                    assert_eq!(
                        discover_corpus_root(Path::new(root), &fs),
                        Some(PathBuf::from(root)),
                        "root={root} install={install} data={data}"
                    );
                }
            }
        }
    }

    /// The subfolder holding the executable is equally arbitrary.
    #[test]
    fn exe_subfolder_names_are_not_assumed() {
        for sub in ["launcher", "bin", "qa-muse-2.0", "x"] {
            let root = "/home/operator/retro";
            let exe_dir = format!("{root}/{sub}");
            let install_dir = format!("{root}/Quake III Arena");
            let fs = FakeFs::default()
                .with(&exe_dir, &[file("qa-muse")])
                .with(root, &[dir("Quake III Arena"), dir(sub)])
                .with(&install_dir, &[dir("baseq3")]);
            assert_eq!(
                discover_corpus_root(Path::new(&exe_dir), &fs),
                Some(PathBuf::from(root)),
                "sub={sub}"
            );
        }
    }

    #[test]
    fn exe_dir_wins_over_parent() {
        let root = "/home/operator/retro";
        let fs = FakeFs::default()
            .with("/home/operator/retro/portable", &[dir("id1")])
            .with(root, &[dir("Quake"), dir("portable")])
            .with("/home/operator/retro/Quake", &[dir("id1")]);
        assert_eq!(
            discover_corpus_root(Path::new("/home/operator/retro/portable"), &fs),
            Some(PathBuf::from("/home/operator/retro/portable"))
        );
    }

    #[test]
    fn normalized_corpus_layout_resolves() {
        let fs = FakeFs::default()
            .with("/corpus", &[dir("q1"), dir("q2"), dir("q3a")])
            .with("/corpus/q1", &[dir("id1")])
            .with("/corpus/q2", &[dir("rerelease")])
            .with("/corpus/q2/rerelease", &[dir("baseq2")])
            .with("/corpus/q3a", &[dir("baseq3")]);
        assert_eq!(
            discover_corpus_root(Path::new("/corpus"), &fs),
            Some(PathBuf::from("/corpus"))
        );
    }

    #[test]
    fn marker_names_match_case_insensitively() {
        let fs = FakeFs::default()
            .with("/mnt/archive/fps", &[dir("Quake2")])
            .with("/mnt/archive/fps/Quake2", &[dir("BaseQ2")]);
        assert_eq!(
            discover_corpus_root(Path::new("/mnt/archive/fps"), &fs),
            Some(PathBuf::from("/mnt/archive/fps"))
        );
        let fs = FakeFs::default().with("/mnt/archive/fps", &[dir("ID1")]);
        assert!(looks_like_content_dir(Path::new("/mnt/archive/fps"), &fs));
    }

    #[test]
    fn loose_archive_next_to_exe_resolves() {
        let fs = FakeFs::default().with("/home/operator/retro/id1", &[file("pak0.pak"), file("qa-muse")]);
        assert!(looks_like_content_dir(Path::new("/home/operator/retro/id1"), &fs));
        let fs = FakeFs::default().with("/home/operator/retro/baseq3", &[file("pak0.pk3")]);
        assert!(looks_like_content_dir(Path::new("/home/operator/retro/baseq3"), &fs));
    }

    #[test]
    fn nested_archives_do_not_resolve() {
        let fs = FakeFs::default()
            .with("/home/operator/retro", &[dir("backup")])
            .with("/home/operator/retro/backup", &[file("pak0.pak")]);
        assert!(!looks_like_content_dir(Path::new("/home/operator/retro"), &fs));
    }

    #[test]
    fn unknown_layout_resolves_nothing() {
        let fs = FakeFs::default()
            .with("/opt/qa-muse", &[file("qa-muse")])
            .with("/opt", &[dir("qa-muse"), dir("documents")]);
        assert_eq!(discover_corpus_root(Path::new("/opt/qa-muse"), &fs), None);
        assert_eq!(discover_corpus_root(Path::new("/missing"), &fs), None);
    }

    #[test]
    fn explicit_input_wins_over_discovery() {
        let fs = FakeFs::default().with("/home/operator/retro", &[dir("id1")]);
        assert_eq!(
            resolve_corpus_root(
                Some("/home/operator/pinned"),
                Some(Path::new("/home/operator/retro")),
                &fs
            ),
            "/home/operator/pinned"
        );
    }

    #[test]
    fn blank_explicit_input_counts_as_unset() {
        let fs = FakeFs::default().with("/home/operator/retro", &[dir("id1")]);
        assert_eq!(
            resolve_corpus_root(Some("   "), Some(Path::new("/home/operator/retro")), &fs),
            "/home/operator/retro"
        );
    }

    #[test]
    fn exe_dir_is_the_fallback_without_markers() {
        let fs = FakeFs::default()
            .with("/opt/qa-muse", &[file("qa-muse")])
            .with("/opt", &[dir("qa-muse")]);
        assert_eq!(
            resolve_corpus_root(None, Some(Path::new("/opt/qa-muse")), &fs),
            "/opt/qa-muse"
        );
    }

    #[test]
    fn working_directory_covers_unknown_exe_path() {
        let fs = FakeFs::default();
        let cwd = std::env::current_dir().expect("working directory");
        assert_eq!(resolve_corpus_root(None, None, &fs), cwd.to_string_lossy().into_owned());
    }

    #[test]
    fn content_directories_resolve_each_item() {
        let fs = FakeFs::default().with("/home/operator/retro", &[dir("id1")]);
        let dirs = ContentDirectories {
            corpus_root: None,
            user_content_root: Some("/home/operator/saves".to_string()),
        };
        assert_eq!(
            dirs.corpus_root(Some(Path::new("/home/operator/retro")), &fs),
            "/home/operator/retro"
        );
        assert_eq!(
            dirs.user_content_root("/home/operator/.local/share/content".to_string()),
            "/home/operator/saves"
        );

        let dirs = ContentDirectories::default();
        assert_eq!(
            dirs.user_content_root("/home/operator/.local/share/content".to_string()),
            "/home/operator/.local/share/content"
        );
        assert_eq!(dirs.corpus_root(Some(Path::new("/opt/qa-muse")), &fs), "/opt/qa-muse");
    }

    #[test]
    fn fs_probe_reports_missing_dirs_as_empty() {
        let probe = FsProbe;
        assert!(probe.children(Path::new("/missing-qa-muse-dir")).is_empty());
        assert!(!looks_like_content_dir(Path::new("/missing-qa-muse-dir"), &probe));
    }
}
