//! Q3 native package reference reporting over the already mounted sources.
use super::*;
use qa_core::{random::Random, text::FixedText};
use std::fmt::Write;

const GENERAL: u8 = 1;
const UI: u8 = 2;
const CGAME: u8 = 4;
const QAGAME: u8 = 8;

pub(super) fn same_account(a: Option<&Arc<References>>, b: Option<&Arc<References>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        _ => false,
    }
}
struct State {
    flags: Box<[u8]>,
    random: Random,
    loose: u32,
}
pub(super) struct References {
    feed: u32,
    pure_checksums: Box<[Option<u32>]>,
    state: Mutex<State>,
    revision: AtomicU64,
}
impl std::fmt::Debug for References {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("References")
            .field("feed", &self.feed)
            .field("sources", &self.pure_checksums.len())
            .finish_non_exhaustive()
    }
}
impl SearchView {
    /// CL_Vid_Restart clears only UI/CGAME bits. General asset references and
    /// other connections' accounts survive the renderer instance replacement.
    pub fn clear_q3_renderer_references(&self, vfs: &Vfs) -> Result<(), VfsError> {
        if !vfs.accepts_search(self) {
            return Err(VfsError::Handle);
        }
        let references = self.references.as_ref().ok_or(VfsError::Handle)?;
        let mut state = references.state.lock().map_err(|_| VfsError::Handle)?;
        let mut changed = false;
        for flags in &mut state.flags {
            changed |= *flags & (UI | CGAME) != 0;
            *flags &= !(UI | CGAME);
        }
        if changed {
            references.revision.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }

    pub(super) fn record_open(&self, entry: &Entry, requested: &[u8]) -> Result<(), VfsError> {
        let Some(references) = &self.references else {
            return Ok(());
        };
        let mut state = references.state.lock().map_err(|_| VfsError::Handle)?;
        let Some(checksum) = references.pure_checksums.get(entry.mount) else {
            return Err(VfsError::Handle);
        };
        if checksum.is_some() {
            let flags = state.flags.get_mut(entry.mount).ok_or(VfsError::Handle)?;
            let before = *flags;
            let contains = |word: &[u8]| requested.windows(word.len()).any(|part| part == word);
            if *flags & GENERAL == 0
                && ![
                    b".shader".as_slice(),
                    b".txt",
                    b".cfg",
                    b".config",
                    b".bot",
                    b".arena",
                    b".menu",
                ]
                .iter()
                .any(|suffix| policy::has_suffix(requested, suffix))
                && !contains(b"levelshots")
            {
                *flags |= GENERAL;
            }
            for (name, flag) in [
                (b"cgame.qvm".as_slice(), CGAME),
                (b"ui.qvm", UI),
                (b"qagame.qvm", QAGAME),
            ] {
                if contains(name) {
                    *flags |= flag;
                }
            }
            if *flags != before {
                references.revision.fetch_add(1, Ordering::Relaxed);
            }
        } else if !policy::Q3_LOOSE_SUFFIXES
            .iter()
            .any(|suffix| policy::has_suffix(requested, suffix))
        {
            // Native fs_fakeChkSum is random wire rejection data for loose
            // assets. It is never computed from content or used as identity.
            state.loose = state.random.next_u32();
            references.revision.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }
    /// FS_ReferencedPakPureChecksums, with the native cp prefix. No file data
    /// is read or checked again here; feed-specific words were prepared at load.
    pub fn q3_reference_command<const N: usize>(
        &self,
        vfs: &Vfs,
        server_id: i32,
        require_modules: bool,
        output: &mut FixedText<N>,
    ) -> Result<(), VfsError> {
        if !vfs.accepts_search(self) {
            return Err(VfsError::Handle);
        }
        let references = self.references.as_ref().ok_or(VfsError::Handle)?;
        let order = &self.ranks.as_ref().ok_or(VfsError::Handle)?.mounts;
        let state = references.state.lock().map_err(|_| VfsError::Handle)?;
        let current = |id: MountId| {
            vfs.mounts
                .get(id.0 as usize)
                .is_some_and(|mount| mount.active)
        };
        if require_modules
            && [CGAME, UI].iter().any(|flag| {
                !order
                    .iter()
                    .any(|id| current(*id) && state.flags[id.0 as usize] & flag != 0)
            })
        {
            return Err(VfsError::Missing);
        }
        let result = (|| {
            output.clear();
            write!(output, "cp {server_id} ")?;
            let mut checksum = references.feed;
            let mut count = 0;
            for flag in [CGAME, UI, GENERAL] {
                if flag == GENERAL {
                    output.append_bytes(b"@ ")?;
                }
                for &id in order {
                    if !current(id) || state.flags[id.0 as usize] & flag == 0 {
                        continue;
                    }
                    let Some(pure) = references.pure_checksums[id.0 as usize] else {
                        continue;
                    };
                    write!(output, "{} ", pure as i32)?;
                    if flag != GENERAL {
                        break;
                    }
                    checksum ^= pure;
                    count += 1;
                }
                if state.loose != 0 {
                    write!(output, "{} ", state.loose as i32)?;
                }
            }
            write!(output, "{}", (checksum ^ count) as i32)
        })();
        if result.is_err() {
            output.clear();
            return Err(VfsError::Limit);
        }
        Ok(())
    }
}
impl Vfs {
    /// A new connection/restart owns a new account. Including this ownership in
    /// selection equality makes warm cache registrations perform its first open.
    pub fn track_q3_references(
        &self,
        view: Option<&SearchView>,
        checksum_feed: u32,
        random: Random,
    ) -> Result<SearchView, VfsError> {
        let mut order = Vec::new();
        self.mount_order(view, &mut order)?;
        let pure_checksums = (0..self.mounts.len())
            .map(|index| {
                self.q3_package_checksums(MountId(index as u32))
                    .map(|checksums| checksums.pure_checksum(checksum_feed))
            })
            .collect();
        let mut selected = self.read_view(view).retain()?;
        // Freeze source membership for this native package account.
        selected.ranks = self.search_view(order.iter().copied())?.ranks;
        selected.references = Some(Arc::new(References {
            feed: checksum_feed,
            pure_checksums,
            revision: AtomicU64::new(0),
            state: Mutex::new(State {
                flags: vec![0; self.mounts.len()].into_boxed_slice(),
                random,
                loose: 0,
            }),
        }));
        Ok(selected)
    }
}

impl ReadView<'_> {
    /// Semantic reference changes only. Probes and reports never advance it.
    pub fn q3_reference_revision(self) -> Result<u64, VfsError> {
        if self
            .search
            .is_some_and(|view| !self.vfs.accepts_search(view))
        {
            return Err(VfsError::Handle);
        }
        Ok(self
            .search
            .and_then(|view| view.references.as_ref())
            .map_or(0, |references| references.revision.load(Ordering::Relaxed)))
    }
    fn q3_package_mounts(self, order: &mut Vec<MountId>) -> Result<(), VfsError> {
        if self
            .search
            .is_some_and(|view| !self.vfs.accepts_search(view))
        {
            return Err(VfsError::Handle);
        }
        if order.capacity() < self.mounts().count() {
            return Err(VfsError::Capacity);
        }
        self.vfs.mount_order(self.search, order)?;
        order.retain(|&id| {
            self.search.is_none_or(|view| view.allows_archive(id))
                && self.vfs.q3_package_checksums(id).is_some()
        });
        Ok(())
    }

    /// FS_LoadedPak{Checksums,Names} and FS_ReferencedPak{Checksums,Names}.
    /// Values project the selected mounted metadata into native systeminfo;
    /// neither a probe nor this report adds a package reference.
    pub fn q3_package_info(
        self,
        pure: bool,
        info: &mut [u8],
        length: &mut usize,
        rules: qa_core::text::InfoRules,
        mounts: &mut Vec<MountId>,
    ) -> Result<(), VfsError> {
        let mut lists: [FixedText<{ qa_core::configstrings::MAX_TEXT - 1 }>; 4] =
            std::array::from_fn(|_| FixedText::default());
        let references = self.search.and_then(|view| view.references.as_ref());
        let state = references
            .map(|references| references.state.lock().map_err(|_| VfsError::Handle))
            .transpose()?;
        self.q3_package_mounts(mounts)?;
        for &id in mounts.iter() {
            let mount = &self.vfs.mounts[id.0 as usize];
            let checks = self.vfs.q3_package_checksums(id).ok_or(VfsError::Handle)?;
            let stem = mount
                .path
                .file_stem()
                .ok_or(VfsError::Path)?
                .as_encoded_bytes();
            let game = mount
                .product
                .and_then(|product| self.vfs.mounts.get(product.0 as usize))
                .map(|product| product.path.as_path())
                .or_else(|| mount.path.parent())
                .and_then(Path::file_name)
                .ok_or(VfsError::Path)?
                .as_encoded_bytes();
            let flags = state
                .as_ref()
                .and_then(|state| state.flags.get(id.0 as usize))
                .copied()
                .unwrap_or(0);
            let referenced = flags != 0
                || !game.get(..6).is_some_and(|prefix| {
                    qa_core::names::compare_folded(prefix, b"baseq3").is_eq()
                });
            let result = (|| {
                if pure {
                    write!(&mut lists[0], "{} ", checks.checksum() as i32)?;
                    if !lists[1].as_bytes().is_empty() {
                        lists[1].append_bytes(b" ")?;
                    }
                    lists[1].append_bytes(stem)?;
                }
                // Original FS_ReferencedPakNames adds its separator even
                // for an unreferenced baseq3 package following a kept row.
                if !lists[3].as_bytes().is_empty() {
                    lists[3].append_bytes(b" ")?;
                }
                if referenced {
                    write!(&mut lists[2], "{} ", checks.checksum() as i32)?;
                    lists[3].append_bytes(game)?;
                    lists[3].append_bytes(b"/")?;
                    lists[3].append_bytes(stem)?;
                }
                Ok::<_, std::fmt::Error>(())
            })();
            result.map_err(|_| VfsError::Limit)?;
        }
        for (name, value) in [
            b"sv_paks".as_slice(),
            b"sv_pakNames",
            b"sv_referencedPaks",
            b"sv_referencedPakNames",
        ]
        .into_iter()
        .zip(&lists)
        {
            qa_core::text::info_set(info, length, name, value.as_bytes(), rules)
                .map_err(|_| VfsError::Limit)?;
        }
        Ok(())
    }

    /// FS_LoadedPakPureChecksums in native source order. Only metadata from the
    /// existing PK3 reader is used, never a second content identity.
    pub fn q3_loaded_pure_checksums(self, feed: u32) -> Result<Vec<u32>, VfsError> {
        let mut mounts = Vec::with_capacity(self.mounts().count());
        self.q3_package_mounts(&mut mounts)?;
        Ok(mounts
            .into_iter()
            .filter_map(|id| {
                self.vfs
                    .q3_package_checksums(id)
                    .map(|pack| pack.pure_checksum(feed))
            })
            .collect())
    }
    /// FS_FileIsInPAK ignores loose-file candidates. It does not open the file
    /// or add a CLIENT reference while the SERVER prepares its expectation.
    pub fn q3_file_pure_checksum(self, name: &[u8], feed: u32) -> Option<u32> {
        if self
            .search
            .is_some_and(|view| !self.vfs.accepts_search(view))
        {
            return None;
        }
        let candidates = self
            .vfs
            .find_candidates(name)?
            .iter()
            .copied()
            .filter(|&slot| {
                self.vfs.entries[slot as usize]
                    .as_ref()
                    .is_some_and(|entry| {
                        self.vfs
                            .q3_package_checksums(MountId(entry.mount as u32))
                            .is_some()
                    })
            });
        let slot = self.vfs.select_candidate(candidates, self.search, true)?;
        let entry = self.vfs.entries[slot as usize].as_ref()?;
        self.vfs
            .q3_package_checksums(MountId(entry.mount as u32))
            .map(|pack| pack.pure_checksum(feed))
    }
}
