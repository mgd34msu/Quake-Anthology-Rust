//! Steam installation inventory (donor `tools/reference/steam.ts`).
//!
//! Observes installed Quake titles and the Proton compatibility runtime.
//! Every probe degrades to an `unavailable` observation instead of failing.

use std::env;
use std::path::PathBuf;

use crate::error::ToolsError;
use crate::reference::schema::{
    CommandObservation, CompatibilityRuntime, FileIdentity, QuakeFamily, ReadObservation, SteamObservation,
    SteamTitleObservation, TitleAvailability,
};

/// An installed Steam title.
#[derive(Debug, Clone)]
pub struct SteamTitle {
    /// Display name.
    pub name: String,
    /// Quake family.
    pub family: QuakeFamily,
    /// Title directory.
    pub path: String,
}

/// Steam `steamapps/common` directory under the current home.
#[must_use]
pub fn steam_common_path() -> PathBuf {
    let home = env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".local/share/Steam/steamapps/common")
}

/// Known Quake titles and their install directories.
#[must_use]
pub fn steam_titles() -> [SteamTitle; 3] {
    let common = steam_common_path();
    [
        SteamTitle {
            name: "Quake".to_owned(),
            family: QuakeFamily::Q1,
            path: common.join("Quake").to_string_lossy().into_owned(),
        },
        SteamTitle {
            name: "Quake 2".to_owned(),
            family: QuakeFamily::Q2,
            path: common.join("Quake 2").to_string_lossy().into_owned(),
        },
        SteamTitle {
            name: "Quake 3 Arena".to_owned(),
            family: QuakeFamily::Q3,
            path: common.join("Quake 3 Arena").to_string_lossy().into_owned(),
        },
    ]
}

/// Whether a file name marks a corpus binary candidate.
///
/// Ports `/\.(?:exe|dll|so(?:\.[a-z0-9_-]+)*)$/i` plus the `q1rets` and
/// `quake3-ts` bare names.
#[must_use]
pub fn is_corpus_binary_candidate(name: &str) -> bool {
    if name == "q1rets" || name == "quake3-ts" {
        return true;
    }
    let lower = name.to_lowercase();
    if lower.ends_with(".exe") || lower.ends_with(".dll") {
        return true;
    }
    if let Some(pos) = lower.rfind(".so") {
        let rest = &lower[pos + 3..];
        if rest.is_empty() {
            return true;
        }
        if let Some(stripped) = rest.strip_prefix('.') {
            return !stripped.is_empty()
                && stripped.split('.').all(|segment| {
                    !segment.is_empty()
                        && segment
                            .chars()
                            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
                });
        }
    }
    false
}

/// Title whose install directory contains `path`, if any.
#[must_use]
pub fn steam_title_for_path(path: &str) -> Option<SteamTitle> {
    steam_titles()
        .into_iter()
        .find(|title| path.starts_with(&format!("{}/", title.path)))
}

/// Availability of a Steam title directory.
///
/// Directory symlinks are not followed, matching the donor reason text.
#[must_use]
pub fn steam_title_availability(title: &SteamTitle) -> SteamTitleObservation {
    let availability = match std::fs::symlink_metadata(&title.path) {
        Ok(details) if details.file_type().is_dir() => TitleAvailability::Present,
        Ok(_) => TitleAvailability::Unavailable {
            reason: "Expected an actual title directory; inventory does not follow directory symlinks.".to_owned(),
        },
        Err(error) => TitleAvailability::Unavailable {
            reason: error.to_string(),
        },
    };
    SteamTitleObservation {
        name: title.name.clone(),
        family: title.family,
        path: title.path.clone(),
        availability,
    }
}

fn read_version(path: &str) -> ReadObservation {
    ReadObservation::read_file(path)
}

/// Observe installed titles and the Proton compatibility runtime.
///
/// Any failure inside the runtime branch degrades to `unavailable`.
pub fn observe_steam(
    identify_file: &dyn Fn(&str) -> Result<FileIdentity, ToolsError>,
    observe_command: &dyn Fn(&[String], &str, Option<u64>) -> Result<CommandObservation, ToolsError>,
    cwd: &str,
) -> SteamObservation {
    let titles = steam_titles().iter().map(steam_title_availability).collect();
    let path = steam_common_path()
        .join("Proton - Experimental")
        .to_string_lossy()
        .into_owned();
    let compatibility_runtime = observe_compatibility_runtime(identify_file, observe_command, cwd, &path)
        .unwrap_or_else(|error| CompatibilityRuntime::Unavailable {
            path: path.clone(),
            reason: error.to_string(),
        });
    SteamObservation {
        common_path: steam_common_path().to_string_lossy().into_owned(),
        titles,
        compatibility_runtime,
        provenance_basis: "Installed title directories and executable filenames determine candidate family and edition; hashes identify local bytes. No vendor-integrity or depot-authenticity claim is made. Unrelated Steam titles, app/account manifests, config files and real compatdata are excluded.".to_owned(),
    }
}

fn observe_compatibility_runtime(
    identify_file: &dyn Fn(&str) -> Result<FileIdentity, ToolsError>,
    observe_command: &dyn Fn(&[String], &str, Option<u64>) -> Result<CommandObservation, ToolsError>,
    cwd: &str,
    path: &str,
) -> Result<CompatibilityRuntime, ToolsError> {
    let selected_files = [
        "proton",
        "version",
        "files/bin/wine",
        "files/bin/wineserver",
        "files/lib/wine/x86_64-unix/wine",
        "files/lib/wine/x86_64-unix/wine64",
        "files/lib/wine/x86_64-unix/wine-preloader",
        "files/lib/wine/x86_64-unix/wine64-preloader",
    ];
    let mut files = Vec::with_capacity(selected_files.len());
    for file in selected_files {
        let joined = PathBuf::from(path).join(file).to_string_lossy().into_owned();
        files.push(identify_file(&joined)?);
    }
    let version_path = PathBuf::from(path).join("version").to_string_lossy().into_owned();
    let version = read_version(&version_path);
    let wine = PathBuf::from(path)
        .join("files/bin/wine")
        .to_string_lossy()
        .into_owned();
    let wine_version = observe_command(&[wine, "--version".to_owned()], cwd, None)?;
    let mut steam_runtime_versions = Vec::new();
    for name in ["SteamLinuxRuntime", "SteamLinuxRuntime_soldier", "SteamLinuxRuntime_4"] {
        let joined = steam_common_path()
            .join(name)
            .join("VERSIONS.txt")
            .to_string_lossy()
            .into_owned();
        steam_runtime_versions.push(read_version(&joined));
    }
    Ok(CompatibilityRuntime::Present {
        path: path.to_owned(),
        files,
        version,
        wine_version,
        steam_runtime_versions,
        launch_policy: [
            "Only wine --version was executed by this inventory. Each title requires a separate observed game launch.",
            "Classic launch candidate: execute the recorded files/bin/wine with a fresh isolated WINEPREFIX, private profile/output directories, a private DISPLAY when required, and the copied game executable/modules with explicitly selected external data.",
            "Do not use the user's existing Steam compatdata or account configuration as a reference profile.",
            "The installed proton launcher performs installation fixups and may create a default prefix before launching. Inventory hashes that launcher but does not execute it in the Steam installation.",
            "The existing Proton launcher is an external reference runtime, not first-party implementation code. First-party launch and capture tooling remains TypeScript.",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_binary_candidates() {
        for name in [
            "quake.exe",
            "GL.DLL",
            "libSDL.so",
            "libx.so.1.2",
            "libx.so.1_2-3",
            "q1rets",
            "quake3-ts",
            ".exe",
        ] {
            assert!(is_corpus_binary_candidate(name), "{name}");
        }
        for name in [
            "quake",
            "mesozoic",
            "a.so.",
            "readme.txt",
            "quake3-tsx",
            "lib.so backup",
            "x.so1",
        ] {
            assert!(!is_corpus_binary_candidate(name), "{name}");
        }
    }

    #[test]
    fn resolves_titles_by_path() {
        let common = steam_common_path().to_string_lossy().into_owned();
        let found = steam_title_for_path(&format!("{common}/Quake 2/baseq2/pak0.pak"));
        assert!(found.is_some_and(|title| title.name == "Quake 2"));
        assert!(steam_title_for_path(&format!("{common}/Other/game.so")).is_none());
        assert!(steam_title_for_path(&common).is_none());
    }
}
