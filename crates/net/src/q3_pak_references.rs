//! Quake III pak reference tracking.
//!
//! Donor provenance: `PakReferenceFlag`, `PakCatalogEntry`,
//! `PakReferenceSnapshot`, `ServerPak`, `ServerPakSet`, `PureSearchPath`,
//! `PakReferences`, `allowedPureLoosePath`, `isPakPure`, and
//! `reorderPurePaks` in `src/network/q3/pak-references.ts` (id Software
//! `files.c`).

use std::collections::HashSet;

use qa_core::cmd::{tokenize_command, Dialect, TextMode};
use qa_core::numeric::native_atoi;
use thiserror::Error;

/// All reference flags.
pub const ALL_REFERENCE_FLAGS: u8 = 0x0f;
/// Big-info payload cap in UTF-16 units.
const BIG_INFO_PAYLOAD_LENGTH: usize = 8191;
/// Maximum parsed search paths.
const MAX_SEARCH_PATHS: usize = 4096;

/// Reference flag (`PakReferenceFlag`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PakReferenceFlag {
    /// General reference.
    General = 0x01,
    /// UI reference.
    Ui = 0x02,
    /// Client-game reference.
    Cgame = 0x04,
    /// Server-game reference.
    Qagame = 0x08,
}

/// Pak-reference failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q3PakError {
    /// Invalid pak state or input, with the donor's message text.
    #[error("{0}")]
    Range(String),
    /// Command-text failure.
    #[error("{0}")]
    Cmd(#[from] qa_core::cmd::CmdError),
    /// Integer-parse failure.
    #[error("{0}")]
    Numeric(#[from] qa_core::numeric::NumericError),
}

/// Mounted pak catalog entry (`PakCatalogEntry`).
///
/// Checksums are `u32`: the donor's signed-or-unsigned-32 validation is
/// enforced by the parameter types at every call site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PakCatalogEntry {
    /// Game directory.
    pub game: String,
    /// Archive basename.
    pub basename: String,
    /// Archive path.
    pub archive_path: String,
    /// Archive checksum.
    pub checksum: u32,
    /// Pure checksum.
    pub pure_checksum: u32,
}

/// Reference snapshot (`PakReferenceSnapshot`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PakReferenceSnapshot {
    /// Catalog entry.
    pub pack: PakCatalogEntry,
    /// Reference flags.
    pub flags: u8,
}

/// Server pak (`ServerPak`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerPak {
    /// Checksum.
    pub checksum: i32,
    /// Pak name, if set.
    pub name: Option<String>,
}

/// Server pak list (`ServerPakSet`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerPakSet {
    sums: Vec<i32>,
    names: Vec<Option<String>>,
}

impl ServerPakSet {
    /// Empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Checksums.
    pub fn checksums(&self) -> &[i32] {
        &self.sums
    }

    /// Parse checksums (`setChecksums`); returns the parsed count.
    pub fn set_checksums(&mut self, text: &str) -> Result<usize, Q3PakError> {
        let argv = tokenize_command(text, Dialect::Q3, TextMode::Source)?.argv;
        self.sums = argv
            .into_iter()
            .take(MAX_SEARCH_PATHS)
            .map(|arg| native_atoi(&arg))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(self.sums.len())
    }

    /// Parse names, clearing the first `clear_count` slots (`setNames`).
    pub fn set_names(&mut self, text: &str, clear_count: Option<usize>) -> Result<(), Q3PakError> {
        let clear = clear_count.unwrap_or(self.sums.len());
        if self.names.len() < clear {
            self.names.resize(clear, None);
        }
        for slot in self.names.iter_mut().take(clear) {
            *slot = None;
        }
        let argv = tokenize_command(text, Dialect::Q3, TextMode::Source)?.argv;
        for (index, name) in argv.into_iter().take(MAX_SEARCH_PATHS).enumerate() {
            if self.names.len() <= index {
                self.names.resize(index + 1, None);
            }
            self.names[index] = Some(name);
        }
        Ok(())
    }

    /// Snapshot the set (`snapshot`).
    pub fn snapshot(&self) -> Vec<ServerPak> {
        self.sums
            .iter()
            .enumerate()
            .map(|(index, checksum)| ServerPak {
                checksum: *checksum,
                name: self.names.get(index).cloned().unwrap_or(None),
            })
            .collect()
    }
}

/// Pure search path (`PureSearchPath`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PureSearchPath<T> {
    /// Directory path.
    Directory {
        /// Path value.
        value: T,
    },
    /// Pak path.
    Pak {
        /// Path value.
        value: T,
        /// Pak checksum.
        checksum: u32,
    },
}

/// Case-insensitive ASCII suffix match (`suffixMatch` in spirit).
///
/// All compared suffixes are ASCII, so byte comparison is exact: a
/// trailing ASCII byte is always a complete character.
fn suffix_equals(path: &str, suffix: &str) -> bool {
    let path = path.as_bytes();
    let suffix = suffix.as_bytes();
    path.len() >= suffix.len() && path[path.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

/// Paths that never take a general reference (`excludesGeneralReference`).
fn excludes_general_reference(path: &str) -> bool {
    suffix_equals(path, ".shader")
        || suffix_equals(path, ".txt")
        || suffix_equals(path, ".cfg")
        || suffix_equals(path, ".config")
        || path.contains("levelshots")
        || suffix_equals(path, ".bot")
        || suffix_equals(path, ".arena")
        || suffix_equals(path, ".menu")
}

/// Loose paths allowed under pure (`allowedPureLoosePath`).
#[must_use]
pub fn allowed_pure_loose_path(path: &str) -> bool {
    suffix_equals(path, ".cfg")
        || suffix_equals(path, ".menu")
        || suffix_equals(path, ".game")
        || suffix_equals(path, ".dm_68")
        || suffix_equals(path, ".dat")
}

/// Whether a pak is pure against a server list (`isPakPure`).
#[must_use]
pub fn is_pak_pure(checksum: u32, server_checksums: &[i32]) -> bool {
    if server_checksums.is_empty() {
        return true;
    }
    let mut matched = false;
    for server in server_checksums {
        if checksum == *server as u32 {
            matched = true;
        }
    }
    matched
}

/// Reorder search paths to match server checksums (`reorderPurePaks`).
pub fn reorder_pure_paks<T: Clone>(search_paths: &[PureSearchPath<T>], server_checksums: &[i32]) -> Vec<PureSearchPath<T>> {
    let mut reordered = search_paths.to_vec();
    let mut insertion = 0;
    for server in server_checksums {
        let pattern = *server as u32;
        let mut index = insertion;
        while index < reordered.len() {
            if matches!(&reordered[index], PureSearchPath::Pak { checksum, .. } if *checksum == pattern) {
                break;
            }
            index += 1;
        }
        if index < reordered.len() {
            let path = reordered.remove(index);
            reordered.insert(insertion, path);
            insertion += 1;
        }
    }
    reordered
}

/// Capped info-string builder (`BigInfoString`).
#[derive(Debug, Clone, Default)]
struct BigInfoString {
    value: String,
}

impl BigInfoString {
    fn append(&mut self, text: &str) {
        let used = self.value.encode_utf16().count();
        if used < BIG_INFO_PAYLOAD_LENGTH {
            let room = BIG_INFO_PAYLOAD_LENGTH - used;
            let units: Vec<u16> = text.encode_utf16().take(room).collect();
            self.value.push_str(&String::from_utf16_lossy(&units));
        }
    }

    fn nonempty(&self) -> bool {
        !self.value.is_empty()
    }

    fn result(self) -> String {
        self.value
    }
}

fn checked_reference_flags(flags: u8) -> Result<u8, Q3PakError> {
    if flags > ALL_REFERENCE_FLAGS {
        return Err(Q3PakError::Range(format!(
            "Pak reference flags must use only the mask {ALL_REFERENCE_FLAGS}"
        )));
    }
    Ok(flags)
}

fn included_in_referenced_report(state: &PakReferenceState) -> bool {
    if state.flags != 0 {
        return true;
    }
    state.pack.game.chars().take(6).collect::<String>().to_lowercase() != "baseq3"
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PakReferenceState {
    pack: PakCatalogEntry,
    flags: u8,
}

/// Pak reference tracker (`PakReferences`).
///
/// The donor keys packs by object identity; here packs key by
/// `archive_path`, which is unique per catalog (duplicates fail like the
/// donor's duplicate-identity check).
pub struct PakReferences {
    states: Vec<PakReferenceState>,
    checksum_feed: u32,
    fake_checksum: i32,
    random: Box<dyn FnMut() -> f64>,
}

impl PakReferences {
    /// Build a tracker over catalog packs.
    pub fn new(
        packs: Vec<PakCatalogEntry>,
        checksum_feed: u32,
        random: impl FnMut() -> f64 + 'static,
    ) -> Result<Self, Q3PakError> {
        let mut references = Self {
            states: Vec::new(),
            checksum_feed,
            fake_checksum: 0,
            random: Box::new(random),
        };
        for pack in packs {
            let state = references.add_state(pack)?;
            references.states.push(state);
        }
        Ok(references)
    }

    /// Checksum feed.
    pub fn checksum_feed(&self) -> u32 {
        self.checksum_feed
    }

    /// Validate and stage a catalog pack (`addState`).
    fn add_state(&mut self, pack: PakCatalogEntry) -> Result<PakReferenceState, Q3PakError> {
        if self.states.iter().any(|state| state.pack.archive_path == pack.archive_path) {
            return Err(Q3PakError::Range(
                "Pak catalog contains the same identity more than once".to_owned(),
            ));
        }
        Ok(PakReferenceState { pack, flags: 0 })
    }

    /// Prepend a catalog pack (`prependPack`).
    pub fn prepend_pack(&mut self, pack: PakCatalogEntry) -> Result<(), Q3PakError> {
        let state = self.add_state(pack)?;
        self.states.insert(0, state);
        Ok(())
    }

    /// Reorder catalog packs (`reorderPacks`).
    pub fn reorder_packs(&mut self, packs: &[PakCatalogEntry]) -> Result<(), Q3PakError> {
        if packs.len() != self.states.len() {
            return Err(Q3PakError::Range(
                "Pak reorder must retain every catalog identity exactly once".to_owned(),
            ));
        }
        let mut seen = HashSet::new();
        for pack in packs {
            if !seen.insert(pack.archive_path.clone()) {
                return Err(Q3PakError::Range(
                    "Pak reorder must retain every catalog identity exactly once".to_owned(),
                ));
            }
        }
        let mut states = Vec::with_capacity(packs.len());
        for pack in packs {
            let Some(state) = self
                .states
                .iter()
                .find(|state| state.pack.archive_path == pack.archive_path)
                .cloned()
            else {
                return Err(Q3PakError::Range("Pak reorder contains an unmounted identity".to_owned()));
            };
            states.push(state);
        }
        self.states = states;
        Ok(())
    }

    /// Carry the loose-open fake checksum across a restart (`retainLooseReference`).
    pub fn retain_loose_reference(&mut self, previous: &PakReferences) {
        self.fake_checksum = previous.fake_checksum;
    }

    /// Record a packed open (`recordPackedOpen`).
    pub fn record_packed_open(&mut self, pack: &PakCatalogEntry, requested_path: &str) -> Result<(), Q3PakError> {
        let Some(state) = self
            .states
            .iter_mut()
            .find(|state| state.pack.archive_path == pack.archive_path)
        else {
            return Err(Q3PakError::Range(
                "Packed open does not belong to this pak catalog".to_owned(),
            ));
        };
        if state.flags & PakReferenceFlag::General as u8 == 0 && !excludes_general_reference(requested_path) {
            state.flags |= PakReferenceFlag::General as u8;
        }
        if state.flags & PakReferenceFlag::Qagame as u8 == 0 && requested_path.contains("qagame.qvm") {
            state.flags |= PakReferenceFlag::Qagame as u8;
        }
        if state.flags & PakReferenceFlag::Cgame as u8 == 0 && requested_path.contains("cgame.qvm") {
            state.flags |= PakReferenceFlag::Cgame as u8;
        }
        if state.flags & PakReferenceFlag::Ui as u8 == 0 && requested_path.contains("ui.qvm") {
            state.flags |= PakReferenceFlag::Ui as u8;
        }
        Ok(())
    }

    /// Record a loose open (`recordLooseOpen`).
    pub fn record_loose_open(&mut self, requested_path: &str) -> Result<(), Q3PakError> {
        if allowed_pure_loose_path(requested_path) {
            return Ok(());
        }
        let random = (self.random)();
        if !random.is_finite() || random < 0.0 || random > 1.0 {
            return Err(Q3PakError::Range(
                "Filesystem random source must return a finite value from 0 through 1".to_owned(),
            ));
        }
        self.fake_checksum = random.trunc() as i32;
        Ok(())
    }

    /// Clear reference flags (`clear`).
    pub fn clear(&mut self, flags: u8) -> Result<(), Q3PakError> {
        let mask = if flags == 0 {
            ALL_REFERENCE_FLAGS
        } else {
            checked_reference_flags(flags)?
        };
        for state in &mut self.states {
            state.flags &= !mask;
        }
        Ok(())
    }

    /// Snapshot the tracker (`snapshot`).
    pub fn snapshot(&self) -> Vec<PakReferenceSnapshot> {
        self.states
            .iter()
            .map(|state| PakReferenceSnapshot {
                pack: state.pack.clone(),
                flags: state.flags,
            })
            .collect()
    }

    /// Loaded pak checksums (`loadedPakChecksums`).
    pub fn loaded_pak_checksums(&self) -> String {
        let mut info = BigInfoString::default();
        for state in &self.states {
            info.append(&format!("{} ", state.pack.checksum as i32));
        }
        info.result()
    }

    /// Loaded pak names (`loadedPakNames`).
    pub fn loaded_pak_names(&self) -> String {
        let mut info = BigInfoString::default();
        for state in &self.states {
            if info.nonempty() {
                info.append(" ");
            }
            info.append(&format!("{}/{}", state.pack.game, state.pack.basename));
        }
        info.result()
    }

    /// Loaded pak pure checksums (`loadedPakPureChecksums`).
    pub fn loaded_pak_pure_checksums(&self) -> String {
        let mut info = BigInfoString::default();
        for state in &self.states {
            info.append(&format!("{} ", state.pack.pure_checksum as i32));
        }
        info.result()
    }

    /// Referenced pak checksums (`referencedPakChecksums`).
    pub fn referenced_pak_checksums(&self) -> String {
        let mut info = BigInfoString::default();
        for state in &self.states {
            if included_in_referenced_report(state) {
                info.append(&format!("{} ", state.pack.checksum as i32));
            }
        }
        info.result()
    }

    /// Referenced pak names (`referencedPakNames`).
    ///
    /// The separator space is appended whenever the builder is nonempty,
    /// even for excluded states, exactly like the donor.
    pub fn referenced_pak_names(&self) -> String {
        let mut info = BigInfoString::default();
        for state in &self.states {
            if info.nonempty() {
                info.append(" ");
            }
            if included_in_referenced_report(state) {
                info.append(&format!("{}/{}", state.pack.game, state.pack.basename));
            }
        }
        info.result()
    }

    /// Referenced pure checksums (`referencedPakPureChecksums`).
    pub fn referenced_pak_pure_checksums(&self) -> String {
        let mut info = BigInfoString::default();
        let mut checksum = self.checksum_feed;
        let mut general_count = 0u32;
        for flag in [PakReferenceFlag::Cgame, PakReferenceFlag::Ui, PakReferenceFlag::General] {
            if flag == PakReferenceFlag::General {
                info.append("@ ");
            }
            for state in &self.states {
                if state.flags & flag as u8 == 0 {
                    continue;
                }
                info.append(&format!("{} ", state.pack.pure_checksum as i32));
                if flag == PakReferenceFlag::Cgame || flag == PakReferenceFlag::Ui {
                    break;
                }
                checksum ^= state.pack.pure_checksum;
                general_count += 1;
            }
            if self.fake_checksum != 0 {
                info.append(&format!("{} ", self.fake_checksum));
            }
        }
        checksum ^= general_count;
        info.append(&(checksum as i32).to_string());
        info.result()
    }

    /// Game pure checksum (`gamePureChecksum`).
    pub fn game_pure_checksum(&self) -> String {
        let mut info = String::new();
        for state in &self.states {
            if state.flags & PakReferenceFlag::Qagame as u8 != 0 {
                info = (state.pack.checksum as i32).to_string();
            }
        }
        info
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(game: &str, basename: &str, checksum: u32, pure: u32) -> PakCatalogEntry {
        PakCatalogEntry {
            game: game.to_owned(),
            basename: basename.to_owned(),
            archive_path: format!("{game}/{basename}"),
            checksum,
            pure_checksum: pure,
        }
    }

    fn references() -> PakReferences {
        PakReferences::new(
            vec![
                entry("baseq3", "pak0.pk3", 100, 1000),
                entry("baseq3", "pak1.pk3", 200, 2000),
            ],
            7,
            || 0.0,
        )
        .unwrap()
    }

    #[test]
    fn pak_sets_parse_and_snapshot() {
        let mut set = ServerPakSet::new();
        assert_eq!(set.set_checksums("1 -2 3").unwrap(), 3);
        assert_eq!(set.checksums(), &[1, -2, 3]);
        set.set_names("a b", None).unwrap();
        assert_eq!(
            set.snapshot(),
            vec![
                ServerPak {
                    checksum: 1,
                    name: Some("a".to_owned())
                },
                ServerPak {
                    checksum: -2,
                    name: Some("b".to_owned())
                },
                ServerPak {
                    checksum: 3,
                    name: None
                },
            ]
        );
        set.set_names("z", Some(3)).unwrap();
        assert_eq!(set.snapshot()[0].name.as_deref(), Some("z"));
        assert_eq!(set.snapshot()[1].name, None);
    }

    #[test]
    fn pure_matching_follows_donor() {
        assert!(is_pak_pure(42, &[]));
        assert!(is_pak_pure(0xFFFF_FFFF, &[-1]));
        assert!(is_pak_pure(7, &[8, 7]));
        assert!(!is_pak_pure(7, &[8]));
        assert!(allowed_pure_loose_path("maps/q3dm1.cfg"));
        assert!(allowed_pure_loose_path("demos/x.dm_68"));
        assert!(!allowed_pure_loose_path("maps/q3dm1.bsp"));
    }

    #[test]
    fn search_paths_reorder_to_server_order() {
        let paths = vec![
            PureSearchPath::Pak {
                value: "a",
                checksum: 1,
            },
            PureSearchPath::Directory { value: "dir" },
            PureSearchPath::Pak {
                value: "b",
                checksum: 2,
            },
        ];
        let reordered = reorder_pure_paks(&paths, &[2, 9]);
        assert_eq!(
            reordered,
            vec![
                PureSearchPath::Pak {
                    value: "b",
                    checksum: 2
                },
                PureSearchPath::Pak {
                    value: "a",
                    checksum: 1
                },
                PureSearchPath::Directory { value: "dir" },
            ]
        );
    }

    #[test]
    fn opens_set_reference_flags() {
        let mut refs = references();
        let shader = entry("baseq3", "pak0.pk3", 100, 1000);
        refs.record_packed_open(&shader, "scripts/base.shader").unwrap();
        assert_eq!(refs.snapshot()[0].flags, 0);
        refs.record_packed_open(&shader, "vm/qagame.qvm").unwrap();
        assert_eq!(
            refs.snapshot()[0].flags,
            PakReferenceFlag::General as u8 | PakReferenceFlag::Qagame as u8
        );
        let ui = entry("baseq3", "pak1.pk3", 200, 2000);
        refs.record_packed_open(&ui, "vm/ui.qvm").unwrap();
        refs.record_packed_open(&ui, "vm/cgame.qvm").unwrap();
        assert_eq!(
            refs.snapshot()[1].flags,
            PakReferenceFlag::General as u8 | PakReferenceFlag::Ui as u8 | PakReferenceFlag::Cgame as u8
        );
        assert_eq!(refs.game_pure_checksum(), "100");
        let foreign = entry("mod", "pak9.pk3", 9, 9);
        assert!(refs.record_packed_open(&foreign, "x").is_err());
    }

    #[test]
    fn referenced_names_keep_donor_spacing() {
        let mut refs = PakReferences::new(
            vec![
                entry("mod1", "a.pk3", 1, 11),
                entry("baseq3", "b.pk3", 2, 22),
                entry("mod2", "c.pk3", 3, 33),
            ],
            0,
            || 0.0,
        )
        .unwrap();
        let first = entry("mod1", "a.pk3", 1, 11);
        let third = entry("mod2", "c.pk3", 3, 33);
        refs.record_packed_open(&first, "models/x.md3").unwrap();
        refs.record_packed_open(&third, "models/y.md3").unwrap();
        assert_eq!(refs.referenced_pak_names(), "mod1/a.pk3  mod2/c.pk3");
        assert_eq!(refs.referenced_pak_checksums(), "1 3 ");
        assert_eq!(refs.loaded_pak_names(), "mod1/a.pk3 baseq3/b.pk3 mod2/c.pk3");
    }

    #[test]
    fn referenced_pure_checksums_fold_feed() {
        let mut refs = references();
        let first = entry("baseq3", "pak0.pk3", 100, 1000);
        refs.record_packed_open(&first, "vm/cgame.qvm").unwrap();
        // cgame section: first flagged pure checksum, then general section
        // after "@ " folds the feed: (7 ^ 1000 ^ 1) as signed.
        assert_eq!(refs.referenced_pak_pure_checksums(), "1000 @ 1000 1006");
    }

    #[test]
    fn loose_opens_gate_fake_checksums() {
        let mut refs = PakReferences::new(vec![entry("baseq3", "pak0.pk3", 100, 1000)], 7, || 1.0).unwrap();
        refs.record_loose_open("autoexec.cfg").unwrap();
        assert_eq!(refs.referenced_pak_pure_checksums(), "@ 7");
        refs.record_loose_open("maps/q3dm1.bsp").unwrap();
        assert_eq!(refs.referenced_pak_pure_checksums(), "1 1 @ 1 7");
        let mut bad = PakReferences::new(Vec::new(), 0, || f64::NAN).unwrap();
        assert!(bad.record_loose_open("maps/q3dm1.bsp").is_err());
    }

    #[test]
    fn clear_masks_flags() {
        let mut refs = references();
        let first = entry("baseq3", "pak0.pk3", 100, 1000);
        refs.record_packed_open(&first, "vm/qagame.qvm").unwrap();
        refs.clear(PakReferenceFlag::General as u8).unwrap();
        assert_eq!(refs.snapshot()[0].flags, PakReferenceFlag::Qagame as u8);
        refs.clear(0).unwrap();
        assert_eq!(refs.snapshot()[0].flags, 0);
        assert!(refs.clear(0x10).is_err());
    }

    #[test]
    fn catalog_reorders_and_rejects_duplicates() {
        let mut refs = references();
        let first = entry("baseq3", "pak0.pk3", 100, 1000);
        let second = entry("baseq3", "pak1.pk3", 200, 2000);
        refs.reorder_packs(&[second, first.clone()]).unwrap();
        assert_eq!(refs.snapshot()[0].pack.basename, "pak1.pk3");
        assert!(refs.prepend_pack(first.clone()).is_err());
        refs.prepend_pack(entry("mod", "pak9.pk3", 9, 99)).unwrap();
        assert_eq!(refs.snapshot()[0].pack.basename, "pak9.pk3");
        assert!(refs.reorder_packs(&[first]).is_err());
    }
}
