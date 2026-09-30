//! Writable user content directories.
//!
//! Donor: `src/content/user-data.ts`.

use std::path::{Path, PathBuf};

use crate::paths::{path_within_root, PathError};

/// Default writable user content root (`~/.local/share/quake-typescript/content`).
///
/// Follows the `HOME` lookup pattern in `qa-app` options: an absent or empty
/// `HOME` falls back to the working directory.
#[must_use]
pub fn default_user_content_root() -> PathBuf {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() => Path::new(&home).join(".local/share/quake-typescript/content"),
        _ => std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".local/share/quake-typescript/content"),
    }
}

/// Resolve a catalog content directory beneath the writable root
/// (`userProductDirectory`).
///
/// Every family uses its catalog contentDirectory beneath the common
/// writable root.
pub fn user_product_directory(user_content_root: &Path, content_directory: &str) -> Result<PathBuf, PathError> {
    path_within_root(user_content_root, content_directory)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_roots_resolve_and_contain() {
        let root = default_user_content_root();
        assert!(root.ends_with(".local/share/quake-typescript/content"));
        let base = Path::new("/tmp/qa-user");
        assert_eq!(
            user_product_directory(base, "q3/baseq3").unwrap(),
            PathBuf::from("/tmp/qa-user/q3/baseq3")
        );
        assert!(user_product_directory(base, "../escape").is_err());
        assert!(user_product_directory(base, "").is_err());
    }
}
