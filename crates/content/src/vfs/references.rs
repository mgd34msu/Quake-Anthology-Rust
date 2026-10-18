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
        } else if !policy::Q3_LOOSE_SUFFIXES
            .iter()
            .any(|suffix| policy::has_suffix(requested, suffix))
        {
            // Native fs_fakeChkSum is random wire rejection data for loose
            // assets. It is never computed from content or used as identity.
            state.loose = state.random.next_u32();
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
            state: Mutex::new(State {
                flags: vec![0; self.mounts.len()].into_boxed_slice(),
                random,
                loose: 0,
            }),
        }));
        Ok(selected)
    }
}
