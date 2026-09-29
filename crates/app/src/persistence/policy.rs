//! Save policy ported from `src/persistence/save-policy.ts`.
//!
//! Eligibility rules, save-slot path containment, and atomic commit.
//! The commit writes a uniquely-named sibling, fsyncs it, renames it over
//! the slot, and fsyncs the parent directory; the donor additionally
//! anchors the parent through a Linux directory descriptor
//! (`src/platform/files/contained.ts`), which this port replaces with
//! lexical containment plus same-directory rename (documented
//! difference: a hostile actor with rename rights on the save directory
//! itself is outside the threat model).

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::PersistenceError;

/// Why a save is being written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SavePurpose {
    /// User-invoked save.
    Manual,
    /// Automatic checkpoint.
    Autosave,
    /// Internal level-transition snapshot.
    Transition,
}

/// Game family for eligibility checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveFamily {
    /// Quake I.
    Q1,
    /// Quake II.
    Q2,
    /// Quake III.
    Q3,
}

/// World authority for eligibility checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveAuthority {
    /// Offline (local) world.
    Offline,
    /// Authoritative network server.
    Server,
    /// Remote client view.
    Remote,
}

/// Match mode for eligibility checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveMode {
    /// Singleplayer.
    Singleplayer,
    /// Cooperative.
    Coop,
    /// Deathmatch.
    Deathmatch,
}

/// World state relevant to save eligibility.
#[derive(Debug, Clone, PartialEq)]
pub struct SaveEligibility {
    /// Game family.
    pub family: SaveFamily,
    /// World authority.
    pub authority: SaveAuthority,
    /// Match mode.
    pub mode: SaveMode,
    /// Whether a world is active.
    pub active: bool,
    /// Whether the world is in intermission.
    pub intermission: bool,
    /// Health of each active player.
    pub player_health: Vec<f64>,
}

/// Explain why a save is unavailable, or `None` when it may proceed.
#[must_use]
pub fn save_unavailable(state: &SaveEligibility, purpose: SavePurpose) -> Option<String> {
    if !state.active {
        return Some("No active world to save.".to_string());
    }
    if purpose == SavePurpose::Transition {
        return if state.authority == SaveAuthority::Remote {
            Some("Only the authoritative world can retain transition state.".to_string())
        } else {
            None
        };
    }
    if state.authority != SaveAuthority::Offline {
        return Some("Save/load unavailable during a network game.".to_string());
    }
    if state.family != SaveFamily::Q3 && state.mode == SaveMode::Deathmatch {
        return Some("Cannot save a deathmatch game.".to_string());
    }
    if state.family != SaveFamily::Q3 && state.intermission {
        return Some("Cannot save during intermission.".to_string());
    }
    if state.player_health.is_empty() {
        return Some("No active player to save.".to_string());
    }
    if state.family != SaveFamily::Q3
        && state
            .player_health
            .iter()
            .any(|health| !health.is_finite() || *health <= 0.0)
    {
        return Some("Cannot save with a dead player.".to_string());
    }
    None
}

fn has_extension(name: &str) -> bool {
    let segment = name.rsplit(['/', '\\']).next().unwrap_or(name);
    matches!(segment.rfind('.'), Some(dot) if dot > 0)
}

/// Resolve a save command argument against the save directory.
pub fn save_command_path(directory: &str, name: &str) -> Result<String, PersistenceError> {
    if name.is_empty() {
        return Err(PersistenceError::BadPath("Enter a save name or file path".to_string()));
    }
    let file = if has_extension(name) {
        name.to_string()
    } else {
        format!("{name}.sav")
    };
    Ok(resolve_path(directory, &file))
}

fn resolve_path(first: &str, second: &str) -> String {
    let second_path = Path::new(second);
    let joined = if second_path.is_absolute() {
        second_path.to_path_buf()
    } else {
        Path::new(first).join(second_path)
    };
    lexical_absolute(&joined)
}

fn lexical_absolute(path: &Path) -> String {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("/"))
            .join(path)
    };
    let mut parts: Vec<String> = Vec::new();
    for component in absolute.components() {
        use std::path::Component;
        match component {
            Component::RootDir => parts.clear(),
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop();
            }
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::Prefix(prefix) => parts.push(prefix.as_os_str().to_string_lossy().into_owned()),
        }
    }
    format!("/{}", parts.join("/"))
}

fn relative_path(directory: &str, path: &str) -> String {
    let dir_parts: Vec<&str> = directory.split('/').filter(|part| !part.is_empty()).collect();
    let path_parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    let common = dir_parts
        .iter()
        .zip(path_parts.iter())
        .take_while(|(left, right)| left == right)
        .count();
    let mut parts = vec![".."; dir_parts.len() - common];
    parts.extend_from_slice(&path_parts[common..]);
    parts.join("/")
}

/// Split a contained file name into validated parts.
pub fn contained_file_parts(name: &str) -> Result<Vec<String>, PersistenceError> {
    if name.is_empty() || name.contains('\\') || name.contains('\0') || name.contains(':') {
        return Err(PersistenceError::BadPath(
            "File path must be relative and stay inside its storage directory".to_string(),
        ));
    }
    let parts: Vec<String> = name.split('/').map(str::to_string).collect();
    if parts.iter().any(|part| part.is_empty() || part == "." || part == "..") {
        return Err(PersistenceError::BadPath(
            "File path must be relative and stay inside its storage directory".to_string(),
        ));
    }
    Ok(parts)
}

/// Validate that a save path stays inside the save directory.
pub fn contained_save_name(directory: &str, path: &str) -> Result<String, PersistenceError> {
    let resolved_dir = lexical_absolute(Path::new(directory));
    let resolved_path = lexical_absolute(Path::new(path));
    let name = relative_path(&resolved_dir, &resolved_path);
    if Path::new(&name).is_absolute() || name == ".." || name.starts_with("../") {
        return Err(PersistenceError::BadPath(format!(
            "Save path is outside the save directory: {resolved_dir}"
        )));
    }
    let parts = contained_file_parts(&name)?;
    if parts.iter().any(|part| {
        let stem = if part.to_lowercase().ends_with(".sav") {
            &part[..part.len() - 4]
        } else {
            part.as_str()
        };
        stem.to_lowercase() == "current"
    }) {
        return Err(PersistenceError::BadPath(
            "The current slot is reserved for transition state".to_string(),
        ));
    }
    if !name.to_lowercase().ends_with(".sav") {
        return Err(PersistenceError::BadPath("Save path must end in .sav".to_string()));
    }
    Ok(name)
}

static SAVE_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_suffix() -> String {
    let counter = SAVE_COUNTER.fetch_add(1, Ordering::SeqCst);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{}-{counter}-{nanos}", std::process::id())
}

/// Atomically write save bytes into a contained slot.
pub fn write_contained_save(directory: &str, path: &str, bytes: &[u8]) -> Result<(), PersistenceError> {
    let name = contained_save_name(directory, path)?;
    fs::create_dir_all(directory)?;
    let target = Path::new(directory).join(&name);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let leaf = target
        .file_name()
        .map(|leaf| leaf.to_string_lossy().into_owned())
        .unwrap_or_default();
    let parent = target
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(directory));
    let temporary = parent.join(format!(".{leaf}.{}.tmp", unique_suffix()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    let write_result: Result<(), std::io::Error> = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, &target)?;
        File::open(&parent)?.sync_all()?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result.map_err(PersistenceError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eligible() -> SaveEligibility {
        SaveEligibility {
            family: SaveFamily::Q2,
            authority: SaveAuthority::Offline,
            mode: SaveMode::Singleplayer,
            active: true,
            intermission: false,
            player_health: vec![100.0],
        }
    }

    #[test]
    fn eligibility_matches_donor_messages() {
        assert_eq!(save_unavailable(&eligible(), SavePurpose::Manual), None);
        assert_eq!(save_unavailable(&eligible(), SavePurpose::Autosave), None);
        assert_eq!(save_unavailable(&eligible(), SavePurpose::Transition), None);
        let mut state = eligible();
        state.active = false;
        assert_eq!(
            save_unavailable(&state, SavePurpose::Manual).as_deref(),
            Some("No active world to save.")
        );
        state = eligible();
        state.authority = SaveAuthority::Remote;
        assert_eq!(
            save_unavailable(&state, SavePurpose::Transition).as_deref(),
            Some("Only the authoritative world can retain transition state.")
        );
        assert_eq!(
            save_unavailable(&state, SavePurpose::Manual).as_deref(),
            Some("Save/load unavailable during a network game.")
        );
        state = eligible();
        state.mode = SaveMode::Deathmatch;
        assert_eq!(
            save_unavailable(&state, SavePurpose::Manual).as_deref(),
            Some("Cannot save a deathmatch game.")
        );
        state.family = SaveFamily::Q3;
        assert_eq!(save_unavailable(&state, SavePurpose::Manual), None);
        state = eligible();
        state.intermission = true;
        assert_eq!(
            save_unavailable(&state, SavePurpose::Manual).as_deref(),
            Some("Cannot save during intermission.")
        );
        state = eligible();
        state.player_health.clear();
        assert_eq!(
            save_unavailable(&state, SavePurpose::Manual).as_deref(),
            Some("No active player to save.")
        );
        state = eligible();
        state.player_health = vec![0.0];
        assert_eq!(
            save_unavailable(&state, SavePurpose::Manual).as_deref(),
            Some("Cannot save with a dead player.")
        );
        state.family = SaveFamily::Q3;
        assert_eq!(save_unavailable(&state, SavePurpose::Manual), None);
    }

    #[test]
    fn save_paths_resolve_and_contain() {
        assert!(save_command_path("/saves", "").is_err());
        assert_eq!(save_command_path("/saves", "slot0").unwrap(), "/saves/slot0.sav");
        assert_eq!(save_command_path("/saves", "slot0.sav").unwrap(), "/saves/slot0.sav");
        assert_eq!(
            contained_save_name("/saves", "/saves/sub/slot.sav").unwrap(),
            "sub/slot.sav"
        );
        assert!(contained_save_name("/saves", "/other/slot.sav").is_err());
        assert!(contained_save_name("/saves", "/saves/current.sav").is_err());
        assert!(contained_save_name("/saves", "/saves/sub/current.sav").is_err());
        assert!(contained_save_name("/saves", "/saves/slot.txt").is_err());
        assert!(contained_file_parts("a/../b").is_err());
        assert!(contained_file_parts("a\\b").is_err());
    }

    #[test]
    fn contained_writes_commit_atomically() {
        let root = std::env::temp_dir().join(format!("qa-save-{}-{}", std::process::id(), unique_suffix()));
        let directory = root.join("saves").to_string_lossy().into_owned();
        let target = format!("{directory}/sub/slot.sav");
        write_contained_save(&directory, &target, b"hello").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"hello");
        write_contained_save(&directory, &target, b"world").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"world");
        let leftovers: Vec<_> = fs::read_dir(root.join("saves").join("sub"))
            .unwrap()
            .filter_map(|entry| entry.ok().map(|entry| entry.file_name()))
            .collect();
        assert_eq!(leftovers.len(), 1);
        assert!(write_contained_save(&directory, "/elsewhere/slot.sav", b"x").is_err());
        let _ = fs::remove_dir_all(&root);
    }
}
