//! Cold search locations for an installed executable's adjacent game data.
use std::{io, path::PathBuf};

pub fn content_roots() -> io::Result<Vec<PathBuf>> {
    let mut roots = vec![std::env::current_dir()?];
    let executable = std::env::current_exe()?;
    if let Some(parent) = executable.parent()
        && roots[0] != parent
    {
        roots.push(parent.to_owned());
    }
    Ok(roots)
}
