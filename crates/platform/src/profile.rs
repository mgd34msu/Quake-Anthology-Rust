//! Cold saved-profile authority. Reading stays in the shared VFS; no directory
//! is created and no profile is modified by selecting its root.
use std::path::PathBuf;

pub fn saved_profile_root() -> Option<PathBuf> {
    let profile = data_root()?.join("quake-anthology/content");
    profile.is_dir().then_some(profile)
}

/// User-created game files have separate authority from saved engine settings.
pub fn user_files_root(namespace: &str) -> Option<PathBuf> {
    if namespace.is_empty()
        || namespace
            .bytes()
            .any(|b| !b.is_ascii_alphanumeric() && b != b'-')
    {
        return None;
    }
    Some(data_root()?.join("quake-anthology/files").join(namespace))
}

fn data_root() -> Option<PathBuf> {
    let root = std::env::var_os("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
        })?;
    Some(root)
}
