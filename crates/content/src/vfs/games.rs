//! Cold native game-directory changes reuse the ordinary mounts and source order.
use super::*;
use crate::products::{self, STOCK};
use qa_core::primitives::ProductId;

fn installed_directory(parent: &Path, name: &[u8]) -> Result<Option<PathBuf>, VfsError> {
    files::directory_entries(parent)?
        .iter()
        .find(|entry| {
            entry.kind != EntryKind::File
                && entry.name.as_encoded_bytes().eq_ignore_ascii_case(name)
        })
        .map(|entry| files::canonicalize(&entry.path).map_err(VfsError::from))
        .transpose()
}

fn game_root(
    root: &Path,
    spec: &products::ProductSpec,
    directory: &[u8],
) -> Result<bool, VfsError> {
    Ok(
        products::root_metadata(root).is_some_and(|metadata| metadata.root_hint == spec.root_hint)
            || root
                .file_name()
                .is_some_and(|name| name.as_encoded_bytes().eq_ignore_ascii_case(directory))
            || match root.parent() {
                Some(parent) => installed_directory(parent, spec.directory)?.is_some(),
                None => false,
            },
    )
}

impl Vfs {
    pub fn source_directory<'a>(&'a self, file: &FileRef) -> Result<&'a Path, VfsError> {
        let entry = self.entry(file)?;
        Ok(self.product_directory(&self.mounts[entry.mount]))
    }

    /// Infer a native directory only from an identified install or its adjacent
    /// base assets. Arbitrary synthetic program roots supply no native default.
    pub fn source_game_directory(
        &self,
        source: &FileRef,
        base: ProductId,
    ) -> Result<Option<qa_core::text::FixedText<MAX_PATH_BYTES>>, VfsError> {
        let spec = STOCK.get(base.0 as usize).ok_or(VfsError::Handle)?;
        let root = self.source_directory(source)?;
        if !game_root(root, spec, b"")? {
            return Ok(None);
        }
        let name = root.file_name().ok_or(VfsError::Path)?.as_encoded_bytes();
        Ok(Some(if name.eq_ignore_ascii_case(spec.directory) {
            qa_core::text::FixedText::default()
        } else {
            products::remote_directory(base, name)?
        }))
    }

    /// Select a server's mod and its stock ancestry inside the caller's installed
    /// roots. Existing readers, archive ordering and FileRefs remain authoritative.
    pub fn game_search(
        &mut self,
        requested: &SearchView,
        base: ProductId,
        game: &[u8],
    ) -> Result<SearchView, VfsError> {
        let directory = products::remote_directory(base, game)?;
        let spec = STOCK.get(base.0 as usize).ok_or(VfsError::Handle)?;
        let fixed = requested.fixed().is_some();
        let mut previous = Vec::new();
        self.mount_order(Some(requested), &mut previous)?;
        let mut parents = Vec::new();
        for id in &previous {
            let mount = &self.mounts[id.0 as usize];
            if mount.settings_only {
                continue;
            }
            let root = self.product_directory(mount);
            let Some(parent) = root.parent() else {
                continue;
            };
            let matching = game_root(root, spec, directory.as_bytes())?;
            if matching && !parents.iter().any(|existing| existing == parent) {
                parents.push(parent.to_path_buf());
            }
        }
        if parents.is_empty() {
            // Synthetic and explicitly unclassified namespaces have no stock
            // install root from which a server can select a sibling mod.
            return if directory.as_bytes().eq_ignore_ascii_case(spec.directory) {
                Ok(requested.clone())
            } else {
                Err(VfsError::Missing)
            };
        }
        let mut names = vec![directory.as_bytes()];
        let mut ancestor = Some(base);
        while let Some(id) = ancestor {
            let spec = STOCK.get(id.0 as usize).ok_or(VfsError::Handle)?;
            if !names
                .iter()
                .any(|name| name.eq_ignore_ascii_case(spec.directory))
            {
                names.push(spec.directory);
            }
            ancestor = spec.base;
        }
        let mut selected_roots = Vec::new();
        let mut found_game = false;
        // Game layers precede every base layer, across all installed roots.
        for (layer, name) in names.iter().enumerate() {
            for parent in &parents {
                let Some(path) = installed_directory(parent, name)? else {
                    continue;
                };
                if fixed
                    && !previous
                        .iter()
                        .any(|id| self.product_directory(&self.mounts[id.0 as usize]) == path)
                {
                    continue;
                }
                found_game |= layer == 0;
                if selected_roots.contains(&path) {
                    continue;
                }
                if !self
                    .mounts()
                    .any(|mount| self.directory_path(mount.id) == Some(path.as_path()))
                {
                    for id in self.mount_product(&path, 0)? {
                        self.mounts[id.0 as usize].default_visible = false;
                    }
                }
                selected_roots.push(path);
            }
        }
        if !found_game
            && !(directory.as_bytes().eq_ignore_ascii_case(spec.directory)
                && !selected_roots.is_empty())
        {
            return Err(VfsError::Missing);
        }
        let mut order = Vec::new();
        for root in &selected_roots {
            let mut product: Vec<_> = self
                .mounts()
                .filter(|mount| {
                    self.product_directory(mount) == root
                        && (!fixed || requested.rank(mount.id).is_some())
                })
                .map(|mount| mount.id)
                .collect();
            product.sort_unstable_by(|a, b| {
                self.mounts[b.0 as usize]
                    .precedence()
                    .cmp(&self.mounts[a.0 as usize].precedence())
            });
            order.extend(product);
        }
        // Keep explicit mixed-content/settings sources. Retired stock mods in
        // this family must not become fallback assets for the next server.
        for id in previous {
            let mount = &self.mounts[id.0 as usize];
            if order.contains(&id) {
                continue;
            }
            let root = self.product_directory(mount);
            if products::root_metadata(root)
                .is_some_and(|metadata| metadata.root_hint == spec.root_hint)
                && root
                    .parent()
                    .is_some_and(|parent| parents.iter().any(|selected| selected == parent))
            {
                continue;
            }
            order.push(id);
        }
        let mut selected = requested.clone();
        selected.ranks = self.search_view(order)?.ranks;
        Ok(selected)
    }

    pub(crate) fn product_directory<'a>(&'a self, mount: &'a MountInfo) -> &'a Path {
        if let Some(root) = mount.product {
            &self.mounts[root.0 as usize].path
        } else if mount.kind == MountKind::Directory {
            &mount.path
        } else {
            mount.path.parent().unwrap_or(&mount.path)
        }
    }
}
