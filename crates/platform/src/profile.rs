//! Cold saved-profile authority. Reading stays in the shared VFS; no directory
//! is created and no profile is modified by selecting its root.
use std::path::PathBuf;

pub fn saved_profile_root() -> Option<PathBuf> {
    let root = std::env::var_os("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
        })?;
    let profile = root.join("quake-anthology/content");
    profile.is_dir().then_some(profile)
}
