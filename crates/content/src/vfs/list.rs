//! Bounded metadata enumeration over the VFS's loaded sources.
use super::*;
use qa_core::stamps::StampSet;

pub struct FileListQuery<'a> {
    pub path: &'a [u8],
    pub extension: &'a [u8],
    pub maximum: usize,
}
pub enum FileListRequest<'a> {
    Empty,
    Files(FileListQuery<'a>),
    Mods {
        base_game: &'a [u8],
        package_suffix: &'a [u8],
        maximum: usize,
    },
}
pub struct FileListName<'a> {
    pub bytes: &'a [u8],
    pub trailing_slash: bool,
}
impl FileListName<'_> {
    pub fn len(&self) -> usize {
        self.bytes.len() + usize::from(self.trailing_slash)
    }
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty() && !self.trailing_slash
    }
}

pub struct FileListing {
    seen: StampSet,
    mounts: Vec<MountId>,
}
impl Default for FileListing {
    fn default() -> Self {
        Self {
            seen: StampSet::new(0),
            mounts: Vec::new(),
        }
    }
}
impl FileListing {
    /// Reserve after a source load/publication, before module callbacks resume.
    pub fn prepare(&mut self, vfs: &Vfs) -> Result<(), VfsError> {
        let names = vfs.names.as_ref().map_or(0, NameTable::len) * 2;
        self.mounts
            .try_reserve(vfs.mounts.len().saturating_sub(self.mounts.len()))
            .map_err(|_| VfsError::Allocation)?;
        if self.seen.len() < names {
            self.seen = StampSet::new(names);
        }
        Ok(())
    }
    fn begin(&mut self, vfs: &Vfs, view: Option<&SearchView>) -> Result<(), VfsError> {
        if view.is_some_and(|view| !vfs.accepts_search(view)) {
            return Err(VfsError::Handle);
        }
        if self.seen.len() < vfs.names.as_ref().map_or(0, NameTable::len) * 2
            || self.mounts.capacity() < vfs.mounts.len()
        {
            return Err(VfsError::Capacity);
        }
        let view = view.and_then(SearchView::fixed);
        self.seen.begin();
        self.mounts.clear();
        self.mounts.extend(
            vfs.mounts()
                .filter(|mount| view.is_none_or(|view| view.rank(mount.id).is_some()))
                .map(|mount| mount.id),
        );
        self.mounts.sort_unstable_by(|a, b| {
            if let Some(view) = view {
                view.rank(*a).cmp(&view.rank(*b))
            } else {
                vfs.mounts[b.0 as usize]
                    .precedence()
                    .cmp(&vfs.mounts[a.0 as usize].precedence())
            }
        });
        Ok(())
    }
    pub fn visit(
        &mut self,
        vfs: &Vfs,
        view: Option<&SearchView>,
        query: FileListQuery<'_>,
        mut emit: impl FnMut(FileListName<'_>) -> bool,
    ) -> Result<usize, VfsError> {
        self.begin(vfs, view)?;
        let mut path = [0; MAX_PATH_BYTES];
        let query_path = query
            .path
            .strip_suffix(b"/")
            .or_else(|| query.path.strip_suffix(b"\\"))
            .unwrap_or(query.path);
        let length = if query_path.is_empty() {
            0
        } else {
            normalize_into(query_path, &mut path)?
        };
        let path = &path[..length];
        let depth = query
            .path
            .iter()
            .filter(|byte| matches!(byte, b'/' | b'\\'))
            .count();
        let directories = query.extension == b"/";
        let mut count = 0;
        for &id in &self.mounts {
            let mount = &vfs.mounts[id.0 as usize];
            let Some(contents) = &mount.contents else {
                continue;
            };
            let mut append = |name: &[u8], original: &[u8], directory: bool| {
                if mount.kind == MountKind::Directory && directory != directories {
                    return true;
                }
                let suffix = if directory && mount.kind != MountKind::Directory {
                    if query.extension.is_empty() {
                        b"".as_slice()
                    } else if let Some(suffix) = query.extension.strip_suffix(b"/") {
                        suffix
                    } else {
                        return true;
                    }
                } else if directory {
                    b"".as_slice()
                } else {
                    query.extension
                };
                if !name
                    .get(name.len().saturating_sub(suffix.len())..)
                    .is_some_and(|end| end.eq_ignore_ascii_case(suffix))
                {
                    return true;
                }
                let Some(relative) =
                    relative_name(name, original, mount.kind, path, depth, directory)
                else {
                    return true;
                };
                let Some(key) = vfs.names.as_ref().and_then(|names| names.find(name)) else {
                    return true;
                };
                let key = key.0 as usize * 2 + usize::from(relative.trailing_slash);
                if self.seen.test_and_set(key) {
                    return true;
                }
                if count == query.maximum || !emit(relative) {
                    return false;
                }
                count += 1;
                true
            };
            let mut files = contents
                .files
                .iter()
                .filter_map(|&slot| vfs.entries.get(slot as usize)?.as_deref())
                .peekable();
            let mut dirs = contents.directories.iter().peekable();
            while files.peek().is_some() || dirs.peek().is_some() {
                let directory = match (files.peek(), dirs.peek()) {
                    (Some(file), Some(dir)) => dir.ordinal < file.ordinal,
                    (None, Some(_)) => true,
                    _ => false,
                };
                let keep = if directory {
                    dirs.next()
                        .is_none_or(|dir| append(&dir.name, &dir.original_name, true))
                } else {
                    files
                        .next()
                        .is_none_or(|file| append(&file.name, &file.original_name, false))
                };
                if !keep {
                    return Ok(count);
                }
            }
        }
        Ok(count)
    }

    /// Enumerate installed products explicitly mounted in this VFS. The caller
    /// chooses its base product and package suffix; no sibling OS scan occurs.
    pub fn visit_mods(
        &mut self,
        vfs: &Vfs,
        view: Option<&SearchView>,
        base_game: &[u8],
        package_suffix: &[u8],
        maximum: usize,
        mut emit: impl FnMut(&[u8], Option<FileRef>) -> bool,
    ) -> Result<usize, VfsError> {
        self.begin(vfs, view)?;
        let mut count = 0;
        for &id in &self.mounts {
            let mount = &vfs.mounts[id.0 as usize];
            if mount.kind != MountKind::Directory || mount.settings_only {
                continue;
            }
            let Some(name) = mount.path.file_name().map(|name| name.as_encoded_bytes()) else {
                continue;
            };
            if name.starts_with(b".") || name.eq_ignore_ascii_case(base_game) {
                continue;
            }
            if !vfs.files_in_mount(id).any(|(_, name)| {
                !name.contains(&b'/')
                    && name
                        .get(name.len().saturating_sub(package_suffix.len())..)
                        .is_some_and(|suffix| suffix.eq_ignore_ascii_case(package_suffix))
            }) {
                continue;
            }
            let Some(key) = vfs.names.as_ref().and_then(|names| names.find_folded(name)) else {
                continue;
            };
            if self.seen.test_and_set(key.0 as usize * 2) {
                continue;
            }
            let description = vfs
                .files_in_mount(id)
                .find(|(_, name)| *name == b"description.txt")
                .map(|(file, _)| file);
            if count == maximum || !emit(name, description) {
                return Ok(count);
            }
            count += 1;
        }
        Ok(count)
    }
}
fn relative_name<'a>(
    name: &'a [u8],
    original: &'a [u8],
    kind: MountKind,
    path: &[u8],
    depth: usize,
    directory: bool,
) -> Option<FileListName<'a>> {
    let first = if path.is_empty() { 0 } else { path.len() + 1 };
    if kind == MountKind::Directory {
        let tail = if path.is_empty() {
            name
        } else {
            name.strip_prefix(path)?.strip_prefix(b"/")?
        };
        if tail.is_empty() || tail.contains(&b'/') {
            return None;
        }
        Some(FileListName {
            bytes: original.get(first..)?,
            trailing_slash: false,
        })
    } else {
        // FS_ReturnPath counts the directory entry's final slash too. Native
        // archives expose up to two levels relative to the caller's raw path.
        let source_depth =
            name.iter().filter(|byte| **byte == b'/').count() + usize::from(directory);
        let parent = if directory {
            name.len()
        } else {
            name.iter().rposition(|byte| *byte == b'/').unwrap_or(0)
        };
        if source_depth.saturating_sub(depth) > 2 || path.len() > parent || !name.starts_with(path)
        {
            return None;
        }
        Some(FileListName {
            bytes: name.get(first..)?,
            trailing_slash: directory,
        })
    }
}
