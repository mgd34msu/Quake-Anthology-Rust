//! Read admission is source metadata on the shared VFS selection.
use super::*;

pub(super) const Q3_LOOSE_SUFFIXES: &[&[u8]] = &[b".cfg", b".menu", b".game", b".dm_68", b".dat"];
pub(super) fn has_suffix(path: &[u8], suffix: &[u8]) -> bool {
    path.get(path.len().saturating_sub(suffix.len())..)
        .is_some_and(|end| end.eq_ignore_ascii_case(suffix))
}
#[derive(Debug, PartialEq, Eq)]
pub(super) struct ReadPolicy {
    archives: Box<[bool]>,
    loose_suffixes: Box<[Box<[u8]>]>,
}
impl SearchView {
    pub(super) fn admits(&self, mount: &MountInfo, path: &[u8], read: bool) -> bool {
        (self.fixed().is_none() || self.rank(mount.id).is_some())
            && (!read || self.allows_read(mount, path))
    }
    pub(super) fn allows_archive(&self, mount: MountId) -> bool {
        self.read_policy.as_ref().is_none_or(|policy| {
            policy
                .archives
                .get(mount.0 as usize)
                .copied()
                .unwrap_or(false)
        })
    }
    fn allows_loose(&self, path: &[u8]) -> bool {
        self.read_policy.as_ref().is_none_or(|policy| {
            policy
                .loose_suffixes
                .iter()
                .any(|suffix| has_suffix(path, suffix))
        })
    }
    pub(super) fn allows_read(&self, mount: &MountInfo, path: &[u8]) -> bool {
        if mount.kind == MountKind::Directory {
            self.allows_loose(path)
        } else {
            self.allows_archive(mount.id)
        }
    }
}
impl Vfs {
    /// Reuse this order for resolution, enumeration and native package policy.
    pub(super) fn mount_order(
        &self,
        view: Option<&SearchView>,
        output: &mut Vec<MountId>,
    ) -> Result<(), VfsError> {
        if view.is_some_and(|view| !self.accepts_search(view)) {
            return Err(VfsError::Handle);
        }
        output.clear();
        if let Some(order) = view.and_then(|view| view.ranks.as_ref()) {
            output.extend(
                order
                    .mounts
                    .iter()
                    .copied()
                    .filter(|id| self.mounts[id.0 as usize].active),
            );
            return Ok(());
        }
        output.extend(self.mounts().map(|mount| mount.id));
        output.sort_unstable_by(|a, b| {
            self.mounts[b.0 as usize]
                .precedence()
                .cmp(&self.mounts[a.0 as usize].precedence())
        });
        Ok(())
    }
    /// Restrict actual reads without hiding names from native existence probes.
    /// An existing policy can only be narrowed. All metadata is prepared at load.
    pub fn restricted_search(
        &self,
        view: Option<&SearchView>,
        archives: &[MountId],
        loose_suffixes: &[&[u8]],
    ) -> Result<SearchView, VfsError> {
        let mut order = Vec::new();
        self.mount_order(view, &mut order)?;
        let mut admitted = vec![false; self.mounts.len()];
        for &id in archives {
            let mount = self
                .mounts
                .get(id.0 as usize)
                .filter(|mount| mount.active)
                .ok_or(VfsError::Handle)?;
            if mount.kind == MountKind::Directory {
                return Err(VfsError::Handle);
            }
            admitted[id.0 as usize] = view.is_none_or(|view| view.allows_archive(id));
        }
        let mut suffixes = Vec::new();
        for &suffix in loose_suffixes {
            if suffix.len() < 2
                || suffix.len() > MAX_PATH_BYTES
                || suffix[0] != b'.'
                || suffix
                    .iter()
                    .any(|byte| matches!(byte, 0 | b'/' | b'\\' | b':'))
            {
                return Err(VfsError::Path);
            }
            if view.is_none_or(|view| view.allows_loose(suffix)) {
                suffixes.push(suffix.iter().map(u8::to_ascii_lowercase).collect());
            }
        }
        suffixes.sort_unstable();
        suffixes.dedup();
        let mut selected = self.search_view(order)?;
        selected.read_policy = Some(Arc::new(ReadPolicy {
            archives: admitted.into_boxed_slice(),
            loose_suffixes: suffixes.into_boxed_slice(),
        }));
        selected.references = view.and_then(|view| view.references.clone());
        Ok(selected)
    }
    /// Apply a connection's admitted order, read policy and reference account
    /// inside the caller's requested namespace. Neither side can widen reads.
    pub fn admitted_search(
        &self,
        requested: Option<&SearchView>,
        admitted: &SearchView,
    ) -> Result<SearchView, VfsError> {
        let mut selected = self.read_view(requested).retain()?;
        let mut order = Vec::new();
        self.mount_order(Some(admitted), &mut order)?;
        order.retain(|&id| selected.fixed().is_none() || selected.rank(id).is_some());
        selected.ranks = self.search_view(order)?.ranks;
        selected.references = admitted.references.clone();
        let Some(policy) = &admitted.read_policy else {
            return Ok(selected);
        };
        let archives: Vec<_> = self
            .mounts()
            .filter(|mount| mount.kind != MountKind::Directory && admitted.allows_archive(mount.id))
            .map(|mount| mount.id)
            .collect();
        let suffixes: Vec<_> = policy
            .loose_suffixes
            .iter()
            .map(|suffix| suffix.as_ref())
            .collect();
        self.restricted_search(Some(&selected), &archives, &suffixes)
    }
    /// Q3 FS_ReorderPurePaks and FS_PakIsPure. Checksums are the existing native
    /// archive wire metadata; the selection itself is keyed by mount identity.
    pub fn q3_search_view(
        &self,
        view: Option<&SearchView>,
        server_checksums: &[u32],
        restrict_loose: bool,
    ) -> Result<SearchView, VfsError> {
        if server_checksums.is_empty() && !restrict_loose {
            return self.read_view(view).retain();
        }
        let mut order = Vec::new();
        self.mount_order(view, &mut order)?;
        let mut insertion = 0;
        for &checksum in server_checksums {
            if let Some(offset) = order[insertion..].iter().position(|&id| {
                self.q3_package_checksums(id)
                    .is_some_and(|pack| pack.checksum() == checksum)
            }) {
                let matched = order.remove(insertion + offset);
                order.insert(insertion, matched);
                insertion += 1;
            }
        }
        let archives: Vec<_> = order
            .iter()
            .copied()
            .filter(|&id| {
                self.mounts[id.0 as usize].kind != MountKind::Directory
                    && view.is_none_or(|view| view.allows_archive(id))
                    && (server_checksums.is_empty()
                        || self
                            .q3_package_checksums(id)
                            .is_some_and(|pack| server_checksums.contains(&pack.checksum())))
            })
            .collect();
        let mut selected = self.restricted_search(view, &archives, Q3_LOOSE_SUFFIXES)?;
        selected.ranks = self.search_view(order)?.ranks;
        Ok(selected)
    }
}
