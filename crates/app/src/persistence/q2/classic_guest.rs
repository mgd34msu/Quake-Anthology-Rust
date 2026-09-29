//! Classic native original saves ported from `src/persistence/q2-classic-guest.ts`.
//!
//! Files written by the selected DLL's API 3 callbacks (not a suspended
//! Windows process): module identity, API/ABI pins, server cvars plus
//! level metadata, the two original files, and visited-level bytes. The
//! [`ClassicOriginalSaveFiles`] overlay scopes callback-owned files to
//! one active operation with Windows creation-disposition semantics;
//! ordinary guest files keep their original owner through the fallback.
//! The donor's `async` loading variants have no port (the host here is
//! synchronous); the sync flows carry the same rules.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use qa_guest::checkpoint::{read_module, write_module, ModuleIdentity};
use qa_world::save::ownership::ProviderCheckpoint;
use qa_world::save::value::{
    arr, boolean, decode_checkpoint_value, encode_checkpoint_value, int, obj, str, SaveJson, SaveReader,
};

use super::super::PersistenceError;

/// Module + map identity for a classic original save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2ClassicOriginalIdentity {
    /// Module.
    pub module: ModuleIdentity,
    /// Map path.
    pub map: String,
}

/// Level metadata: configstrings plus portal states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2ClassicLevelState {
    /// Configstrings.
    pub configstrings: Vec<(i64, String)>,
    /// Portals.
    pub portals: Vec<(i64, bool)>,
}

/// Visited-level bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2ClassicVisitedLevel {
    /// State.
    pub state: Q2ClassicLevelState,
    /// Map path.
    pub map: String,
    /// Level bytes.
    pub level: Vec<u8>,
}

/// Server state: level metadata plus the encoded cvar registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2ClassicOriginalServerState {
    /// State.
    pub state: Q2ClassicLevelState,
    /// Encoded cvars.
    pub cvars: Vec<u8>,
}

/// Classic native original save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2ClassicOriginalSave {
    /// Identity.
    pub identity: Q2ClassicOriginalIdentity,
    /// Autosave flag.
    pub autosave: bool,
    /// Server state.
    pub server: Q2ClassicOriginalServerState,
    /// Game file bytes.
    pub game: Vec<u8>,
    /// Level file bytes.
    pub level: Vec<u8>,
    /// Visited levels.
    pub visited_levels: Vec<Q2ClassicVisitedLevel>,
}

fn read_level_state(reader: SaveReader) -> Result<Q2ClassicLevelState, PersistenceError> {
    Ok(Q2ClassicLevelState {
        configstrings: reader
            .field("configstrings")
            .list(|entry| -> Result<(i64, String), PersistenceError> {
                Ok((entry.field("index").integer(0)?, entry.field("value").string()?))
            })?,
        portals: reader
            .field("portals")
            .list(|entry| -> Result<(i64, bool), PersistenceError> {
                Ok((entry.field("portal").integer(0)?, entry.field("open").boolean()?))
            })?,
    })
}

fn write_level_state(state: &Q2ClassicLevelState) -> Vec<(&'static str, SaveJson)> {
    vec![
        (
            "configstrings",
            arr(state
                .configstrings
                .iter()
                .map(|(index, value)| obj(vec![("index", int(*index)), ("value", str(value))]))
                .collect()),
        ),
        (
            "portals",
            arr(state
                .portals
                .iter()
                .map(|(portal, open)| obj(vec![("portal", int(*portal)), ("open", boolean(*open))]))
                .collect()),
        ),
    ]
}

fn validate_level_state(state: &Q2ClassicLevelState) -> Result<(), PersistenceError> {
    let bad = |message: &str| PersistenceError::BadSave(message.to_string());
    let mut indexes = std::collections::HashSet::new();
    let mut portals = std::collections::HashSet::new();
    if !state.configstrings.iter().all(|(index, _)| indexes.insert(*index))
        || !state.portals.iter().all(|(portal, _)| portals.insert(*portal))
        || state
            .configstrings
            .iter()
            .any(|(index, _)| *index < 0 || *index >= 2080)
        || state.portals.iter().any(|(portal, _)| *portal < 0 || *portal >= 1024)
    {
        return Err(bad("Invalid classic native level metadata"));
    }
    Ok(())
}

fn bad(message: &str) -> PersistenceError {
    PersistenceError::BadSave(message.to_string())
}

fn validate(save: &Q2ClassicOriginalSave, expected: &Q2ClassicOriginalIdentity) -> Result<(), PersistenceError> {
    if save.identity != *expected {
        return Err(bad("Classic native save differs from the selected module, ABI or map"));
    }
    validate_level_state(&save.server.state)?;
    if save.server.cvars.is_empty() {
        return Err(bad("Invalid classic native server state"));
    }
    let mut visited = std::collections::HashSet::new();
    for level in &save.visited_levels {
        let valid = level.map.starts_with("maps/")
            && level.map.ends_with(".bsp")
            && !level
                .map
                .bytes()
                .any(|byte| byte < 32 || byte == 127 || byte == b'\\' || byte == b':')
            && level.map.len() > 9
            && !level
                .map
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
            && level.map != save.identity.map
            && visited.insert(level.map.clone())
            && !level.level.is_empty();
        if !valid {
            return Err(bad("Invalid classic native visited level"));
        }
        validate_level_state(&level.state)?;
    }
    decode_checkpoint_value(&save.server.cvars).map_err(|_| bad("Invalid classic native server state"))?;
    if save.game.is_empty() || save.level.is_empty() {
        return Err(bad("Classic native save requires both original files"));
    }
    Ok(())
}

/// Encode a classic original save as a provider record.
pub fn encode_q2_classic_original_save(save: &Q2ClassicOriginalSave) -> Result<ProviderCheckpoint, PersistenceError> {
    validate(save, &save.identity)?;
    let mut members = vec![
        ("module", write_module(&save.identity.module)),
        ("map", str(&save.identity.map)),
        ("api", obj(vec![("kind", str("q2-classic-game")), ("version", int(3))])),
        ("abi", str("windows-i386")),
        ("autosave", boolean(save.autosave)),
    ];
    let mut server = write_level_state(&save.server.state);
    server.push(("cvars", SaveJson::Bytes(save.server.cvars.clone())));
    members.push(("server", obj(server)));
    members.push(("game", SaveJson::Bytes(save.game.clone())));
    members.push(("level", SaveJson::Bytes(save.level.clone())));
    members.push((
        "visitedLevels",
        arr(save
            .visited_levels
            .iter()
            .map(|level| {
                let mut members = vec![("version", int(1))];
                members.extend(write_level_state(&level.state));
                members.push(("map", str(&level.map)));
                members.push(("level", SaveJson::Bytes(level.level.clone())));
                obj(members)
            })
            .collect()),
    ));
    Ok(ProviderCheckpoint {
        provider: save.identity.module.id.clone(),
        schema: "q2:classic-native-original".to_string(),
        version: 1,
        bytes: encode_checkpoint_value(&obj(members)),
    })
}

/// Decode a classic original save provider record.
pub fn decode_q2_classic_original_save(
    record: &ProviderCheckpoint,
    expected: &Q2ClassicOriginalIdentity,
) -> Result<Q2ClassicOriginalSave, PersistenceError> {
    if record.schema != "q2:classic-native-original" || record.version != 1 || record.provider != expected.module.id {
        return Err(bad("Unsupported classic native source save provider"));
    }
    let payload = decode_checkpoint_value(&record.bytes)?;
    let reader = SaveReader::at(&payload, "q2.classic-native-original");
    let api = reader.field("api");
    api.field("kind").literal_str("q2-classic-game")?;
    api.field("version").literal_i64(3)?;
    reader.field("abi").literal_str("windows-i386")?;
    let state = reader.field("server");
    let server = Q2ClassicOriginalServerState {
        state: read_level_state(state.clone())?,
        cvars: state.field("cvars").bytes()?,
    };
    let visits = reader.field("visitedLevels");
    let visited_levels = if visits.is_missing() {
        Vec::new()
    } else {
        visits.list(|entry| -> Result<Q2ClassicVisitedLevel, PersistenceError> {
            entry.field("version").literal_i64(1)?;
            Ok(Q2ClassicVisitedLevel {
                state: read_level_state(entry.clone())?,
                map: entry.field("map").string()?,
                level: entry.field("level").bytes()?,
            })
        })?
    };
    let save = Q2ClassicOriginalSave {
        identity: Q2ClassicOriginalIdentity {
            module: read_module(reader.field("module"))?,
            map: reader.field("map").string()?,
        },
        autosave: reader.field("autosave").boolean()?,
        server,
        game: reader.field("game").bytes()?,
        level: reader.field("level").bytes()?,
        visited_levels,
    };
    validate(&save, expected)?;
    Ok(save)
}

/// Open mode for overlay files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicOpenMode {
    /// Readable.
    pub read: bool,
    /// Writable.
    pub write: bool,
    /// Windows creation disposition (1-5).
    pub creation: u32,
}

/// Callback-owned file handle.
pub trait ClassicGuestFile {
    /// Read a range.
    fn read(&mut self, offset: usize, length: usize) -> Result<Vec<u8>, PersistenceError>;
    /// Write at an offset, growing with zeros.
    fn write(&mut self, offset: usize, bytes: &[u8]) -> Result<usize, PersistenceError>;
    /// Current size.
    fn size(&mut self) -> Result<usize, PersistenceError>;
    /// Resize (grows with zeros).
    fn truncate(&mut self, length: usize) -> Result<(), PersistenceError>;
    /// Flush (no-op).
    fn flush(&mut self) -> Result<(), PersistenceError>;
    /// Close and retire the handle.
    fn close(&mut self);
    /// Whether closed.
    fn is_closed(&self) -> bool;
}

/// In-memory file for fallback results and tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryFile {
    bytes: Vec<u8>,
    closed: bool,
    read: bool,
    write: bool,
}

impl MemoryFile {
    /// New in-memory file.
    #[must_use]
    pub fn new(bytes: Vec<u8>, read: bool, write: bool) -> Self {
        Self {
            bytes,
            closed: false,
            read,
            write,
        }
    }
}

impl ClassicGuestFile for MemoryFile {
    fn read(&mut self, offset: usize, length: usize) -> Result<Vec<u8>, PersistenceError> {
        if self.closed {
            return Err(bad("Native save handle is retired"));
        }
        if !self.read {
            return Err(bad("Native save file is not readable"));
        }
        let end = offset
            .checked_add(length)
            .ok_or_else(|| bad("Invalid native save file range"))?;
        if offset > self.bytes.len() {
            return Ok(Vec::new());
        }
        Ok(self.bytes[offset..end.min(self.bytes.len())].to_vec())
    }

    fn write(&mut self, offset: usize, bytes: &[u8]) -> Result<usize, PersistenceError> {
        if self.closed {
            return Err(bad("Native save handle is retired"));
        }
        if !self.write {
            return Err(bad("Native save file is not writable"));
        }
        let end = offset
            .checked_add(bytes.len())
            .ok_or_else(|| bad("Invalid native save file range"))?;
        if end > self.bytes.len() {
            self.bytes.resize(end, 0);
        }
        self.bytes[offset..end].copy_from_slice(bytes);
        Ok(bytes.len())
    }

    fn size(&mut self) -> Result<usize, PersistenceError> {
        if self.closed {
            return Err(bad("Native save handle is retired"));
        }
        Ok(self.bytes.len())
    }

    fn truncate(&mut self, length: usize) -> Result<(), PersistenceError> {
        if self.closed {
            return Err(bad("Native save handle is retired"));
        }
        if !self.write {
            return Err(bad("Native save file is not writable"));
        }
        self.bytes.resize(length, 0);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), PersistenceError> {
        if self.closed {
            return Err(bad("Native save handle is retired"));
        }
        Ok(())
    }

    fn close(&mut self) {
        self.closed = true;
    }

    fn is_closed(&self) -> bool {
        self.closed
    }
}

struct Entry {
    bytes: Vec<u8>,
    read: bool,
    open: usize,
}

struct Operation {
    kind: OperationKind,
    game: String,
    level: String,
    entries: HashMap<String, Rc<RefCell<Entry>>>,
    generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OperationKind {
    Capture,
    Restore,
    CaptureLevel,
    RestoreLevel,
}

/// Overlay-owned file handle.
pub struct ClassicOriginalFile {
    entry: Rc<RefCell<Entry>>,
    epoch: Rc<Cell<u64>>,
    generation: u64,
    mode: ClassicOpenMode,
    closed: bool,
}

impl ClassicGuestFile for ClassicOriginalFile {
    fn read(&mut self, offset: usize, length: usize) -> Result<Vec<u8>, PersistenceError> {
        self.live()?;
        if !self.mode.read {
            return Err(bad("Native save file is not readable"));
        }
        let end = offset
            .checked_add(length)
            .ok_or_else(|| bad("Invalid native save file range"))?;
        let mut entry = self.entry.borrow_mut();
        if length > 0 && offset < entry.bytes.len() {
            entry.read = true;
        }
        if offset > entry.bytes.len() {
            return Ok(Vec::new());
        }
        Ok(entry.bytes[offset..end.min(entry.bytes.len())].to_vec())
    }

    fn write(&mut self, offset: usize, bytes: &[u8]) -> Result<usize, PersistenceError> {
        self.live()?;
        if !self.mode.write {
            return Err(bad("Native save file is not writable"));
        }
        let end = offset
            .checked_add(bytes.len())
            .ok_or_else(|| bad("Invalid native save file range"))?;
        let mut entry = self.entry.borrow_mut();
        if end > entry.bytes.len() {
            entry.bytes.resize(end, 0);
        }
        entry.bytes[offset..end].copy_from_slice(bytes);
        Ok(bytes.len())
    }

    fn size(&mut self) -> Result<usize, PersistenceError> {
        self.live()?;
        Ok(self.entry.borrow().bytes.len())
    }

    fn truncate(&mut self, length: usize) -> Result<(), PersistenceError> {
        self.live()?;
        if !self.mode.write {
            return Err(bad("Native save file is not writable"));
        }
        self.entry.borrow_mut().bytes.resize(length, 0);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), PersistenceError> {
        self.live()?;
        Ok(())
    }

    fn close(&mut self) {
        if !self.closed {
            self.closed = true;
            let open = self.entry.borrow().open;
            self.entry.borrow_mut().open = open.saturating_sub(1);
        }
    }

    fn is_closed(&self) -> bool {
        self.closed
    }
}

impl Drop for ClassicOriginalFile {
    fn drop(&mut self) {
        self.close();
    }
}

impl ClassicOriginalFile {
    fn live(&self) -> Result<(), PersistenceError> {
        if self.closed || self.epoch.get() != self.generation {
            return Err(bad("Native save handle is retired"));
        }
        Ok(())
    }
}

const ROOT: &str = "__qts_original_save/";

fn normalize(path: &str) -> String {
    let slashed = path.replace('\\', "/");
    let mut rest = slashed.as_str();
    while let Some(stripped) = rest.strip_prefix("./") {
        rest = stripped;
    }
    rest.to_string()
}

/// Fallback opener for ordinary guest files outside the overlay root.
type FallbackOpener = Box<dyn Fn(&str, &ClassicOpenMode) -> Option<MemoryFile>>;

/// Overlay scoping callback-owned save files to one active operation.
pub struct ClassicOriginalSaveFiles {
    fallback: Option<FallbackOpener>,
    operation: RefCell<Option<Operation>>,
    epoch: Rc<Cell<u64>>,
}

impl ClassicOriginalSaveFiles {
    /// New overlay with an optional fallback for ordinary guest files.
    #[must_use]
    pub fn new(fallback: Option<FallbackOpener>) -> Self {
        Self {
            fallback,
            operation: RefCell::new(None),
            epoch: Rc::new(Cell::new(1)),
        }
    }

    /// Open a callback-owned file, or box a fallback file.
    pub fn open_file(&self, path: &str, mode: ClassicOpenMode) -> Result<Option<ClassicOpenedFile>, PersistenceError> {
        let normalized = normalize(path);
        if !normalized.starts_with(ROOT) {
            return Ok(self
                .fallback
                .as_ref()
                .and_then(|fallback| fallback(path, &mode))
                .map(ClassicOpenedFile::Fallback));
        }
        let mut operation = self.operation.borrow_mut();
        let Some(operation) = operation.as_mut() else {
            return Err(bad("Native save file is outside the active callback operation"));
        };
        let level_only = matches!(
            operation.kind,
            OperationKind::CaptureLevel | OperationKind::RestoreLevel
        );
        if normalized != operation.game && normalized != operation.level || level_only && normalized != operation.level
        {
            return Err(bad("Native save file is outside the active callback operation"));
        }
        if !(1..=5).contains(&mode.creation) {
            return Err(bad("Invalid native save file disposition"));
        }
        if matches!(operation.kind, OperationKind::Restore | OperationKind::RestoreLevel)
            && (mode.write || mode.creation != 3)
        {
            return Err(bad("Original restore files are read-only"));
        }
        if !mode.write && mode.creation != 3 {
            return Ok(None);
        }
        let exists = operation.entries.contains_key(&normalized);
        if mode.creation == 1 && exists || (mode.creation == 3 || mode.creation == 5) && !exists {
            return Ok(None);
        }
        let entry = operation.entries.entry(normalized).or_insert_with(|| {
            Rc::new(RefCell::new(Entry {
                bytes: Vec::new(),
                read: false,
                open: 0,
            }))
        });
        if mode.creation == 2 || mode.creation == 5 {
            entry.borrow_mut().bytes.clear();
        }
        entry.borrow_mut().open += 1;
        Ok(Some(ClassicOpenedFile::Owned(ClassicOriginalFile {
            entry: Rc::clone(entry),
            epoch: Rc::clone(&self.epoch),
            generation: operation.generation,
            mode,
            closed: false,
        })))
    }

    fn run<T>(
        &self,
        kind: OperationKind,
        seed: Vec<(&str, Vec<u8>)>,
        action: impl FnOnce(&str, &str) -> Result<T, PersistenceError>,
    ) -> Result<T, PersistenceError> {
        if self.operation.borrow().is_some() {
            return Err(bad("Native save callbacks cannot overlap"));
        }
        self.epoch.set(self.epoch.get() + 1);
        let current = self.epoch.get();
        let mut entries = HashMap::new();
        for (key, bytes) in seed {
            let name = if key == "game" {
                format!("{ROOT}game.ssv")
            } else {
                format!("{ROOT}level.sav")
            };
            entries.insert(
                name,
                Rc::new(RefCell::new(Entry {
                    bytes,
                    read: false,
                    open: 0,
                })),
            );
        }
        *self.operation.borrow_mut() = Some(Operation {
            kind,
            game: format!("{ROOT}game.ssv"),
            level: format!("{ROOT}level.sav"),
            entries,
            generation: current,
        });
        let (game, level) = {
            let operation = self.operation.borrow();
            let operation = operation.as_ref().expect("active operation");
            (operation.game.clone(), operation.level.clone())
        };
        let result = action(&game, &level);
        self.operation.borrow_mut().take();
        self.epoch.set(self.epoch.get() + 1);
        result
    }

    fn assert_closed(&self) -> Result<(), PersistenceError> {
        let operation = self.operation.borrow();
        let Some(operation) = operation.as_ref() else {
            return Ok(());
        };
        if operation.entries.values().any(|entry| entry.borrow().open != 0) {
            return Err(bad("Native save callback retained an open original file"));
        }
        Ok(())
    }

    /// Capture both original files through DLL write callbacks.
    pub fn capture(
        &self,
        identity: &Q2ClassicOriginalIdentity,
        server: Q2ClassicOriginalServerState,
        write: impl FnOnce(&str, &str, bool) -> Result<(), PersistenceError>,
        autosave: bool,
    ) -> Result<Q2ClassicOriginalSave, PersistenceError> {
        self.run(OperationKind::Capture, Vec::new(), |game, level| {
            write(game, level, autosave)?;
            self.assert_closed()?;
            let operation = self.operation.borrow();
            let operation = operation.as_ref().expect("active operation");
            let game_bytes = operation
                .entries
                .get(&operation.game)
                .map(|entry| entry.borrow().bytes.clone());
            let level_bytes = operation
                .entries
                .get(&operation.level)
                .map(|entry| entry.borrow().bytes.clone());
            match (game_bytes, level_bytes) {
                (Some(game), Some(level)) => {
                    let save = Q2ClassicOriginalSave {
                        identity: identity.clone(),
                        autosave,
                        server,
                        game,
                        level,
                        visited_levels: Vec::new(),
                    };
                    validate(&save, identity)?;
                    Ok(save)
                }
                _ => Err(bad("Native save callbacks did not write both original files")),
            }
        })
    }

    /// Restore both original files through DLL read callbacks.
    pub fn restore(
        &self,
        save: &Q2ClassicOriginalSave,
        expected: &Q2ClassicOriginalIdentity,
        read: impl FnOnce(&str, &str) -> Result<(), PersistenceError>,
    ) -> Result<(), PersistenceError> {
        validate(save, expected)?;
        self.run(
            OperationKind::Restore,
            vec![("game", save.game.clone()), ("level", save.level.clone())],
            |game, level| {
                read(game, level)?;
                self.assert_closed()?;
                let operation = self.operation.borrow();
                let operation = operation.as_ref().expect("active operation");
                if operation.entries.values().any(|entry| !entry.borrow().read) {
                    return Err(bad("Native restore callbacks did not consume both original files"));
                }
                Ok(())
            },
        )
    }

    /// Capture departed level bytes while the DLL stays resident.
    pub fn capture_travel_level(
        &self,
        write: impl FnOnce(&str) -> Result<(), PersistenceError>,
    ) -> Result<Vec<u8>, PersistenceError> {
        self.run(OperationKind::CaptureLevel, Vec::new(), |_, level| {
            write(level)?;
            self.assert_closed()?;
            let operation = self.operation.borrow();
            let operation = operation.as_ref().expect("active operation");
            match operation
                .entries
                .get(&operation.level)
                .map(|entry| entry.borrow().bytes.clone())
            {
                Some(bytes) if !bytes.is_empty() => Ok(bytes),
                _ => Err(bad("Native travel callback did not write a level file")),
            }
        })
    }

    /// Run a travel read against retained level bytes.
    pub fn with_travel_level<T>(
        &self,
        bytes: &[u8],
        read: impl FnOnce(&str) -> Result<T, PersistenceError>,
    ) -> Result<T, PersistenceError> {
        if bytes.is_empty() {
            return Err(bad("Native travel requires original level bytes"));
        }
        self.run(
            OperationKind::RestoreLevel,
            vec![("level", bytes.to_vec())],
            |_, level| {
                let result = read(level)?;
                self.assert_closed()?;
                let operation = self.operation.borrow();
                let operation = operation.as_ref().expect("active operation");
                let consumed = operation
                    .entries
                    .get(&operation.level)
                    .is_some_and(|entry| entry.borrow().read);
                if !consumed {
                    return Err(bad("Native travel callback did not consume the level file"));
                }
                Ok(result)
            },
        )
    }
}

/// Opened callback-owned or fallback file.
pub enum ClassicOpenedFile {
    /// Overlay-owned handle.
    Owned(ClassicOriginalFile),
    /// Fallback file.
    Fallback(MemoryFile),
}

impl ClassicGuestFile for ClassicOpenedFile {
    fn read(&mut self, offset: usize, length: usize) -> Result<Vec<u8>, PersistenceError> {
        match self {
            Self::Owned(file) => file.read(offset, length),
            Self::Fallback(file) => file.read(offset, length),
        }
    }

    fn write(&mut self, offset: usize, bytes: &[u8]) -> Result<usize, PersistenceError> {
        match self {
            Self::Owned(file) => file.write(offset, bytes),
            Self::Fallback(file) => file.write(offset, bytes),
        }
    }

    fn size(&mut self) -> Result<usize, PersistenceError> {
        match self {
            Self::Owned(file) => file.size(),
            Self::Fallback(file) => file.size(),
        }
    }

    fn truncate(&mut self, length: usize) -> Result<(), PersistenceError> {
        match self {
            Self::Owned(file) => file.truncate(length),
            Self::Fallback(file) => file.truncate(length),
        }
    }

    fn flush(&mut self) -> Result<(), PersistenceError> {
        match self {
            Self::Owned(file) => file.flush(),
            Self::Fallback(file) => file.flush(),
        }
    }

    fn close(&mut self) {
        match self {
            Self::Owned(file) => file.close(),
            Self::Fallback(file) => file.close(),
        }
    }

    fn is_closed(&self) -> bool {
        match self {
            Self::Owned(file) => file.is_closed(),
            Self::Fallback(file) => file.is_closed(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_world::save::value::num;

    fn module() -> ModuleIdentity {
        ModuleIdentity {
            id: "q2:gamex86".to_string(),
            artifact_path: "gamex86.dll".to_string(),
            digest: format!("sha256:{}", "0".repeat(64)),
            revision: "3".to_string(),
        }
    }

    fn identity() -> Q2ClassicOriginalIdentity {
        Q2ClassicOriginalIdentity {
            module: module(),
            map: "maps/base1.bsp".to_string(),
        }
    }

    fn server() -> Q2ClassicOriginalServerState {
        Q2ClassicOriginalServerState {
            state: Q2ClassicLevelState {
                configstrings: vec![(0, "g".to_string())],
                portals: vec![(1, true)],
            },
            cvars: encode_checkpoint_value(&obj(vec![("maxclients", num(1.0))])),
        }
    }

    #[test]
    fn original_saves_round_trip() {
        let save = Q2ClassicOriginalSave {
            identity: identity(),
            autosave: false,
            server: server(),
            game: vec![1, 2, 3],
            level: vec![4, 5],
            visited_levels: vec![Q2ClassicVisitedLevel {
                state: Q2ClassicLevelState {
                    configstrings: Vec::new(),
                    portals: Vec::new(),
                },
                map: "maps/base2.bsp".to_string(),
                level: vec![9],
            }],
        };
        let record = encode_q2_classic_original_save(&save).unwrap();
        assert_eq!(record.provider, "q2:gamex86");
        assert_eq!(record.schema, "q2:classic-native-original");
        assert_eq!(decode_q2_classic_original_save(&record, &identity()).unwrap(), save);
        let mut bad = save.clone();
        bad.game.clear();
        assert!(encode_q2_classic_original_save(&bad).is_err());
    }

    #[test]
    fn overlay_scopes_callback_files() {
        let overlay = ClassicOriginalSaveFiles::new(None);
        let save = overlay
            .capture(
                &identity(),
                server(),
                |game, level, _| {
                    for (path, bytes) in [(game, vec![7u8]), (level, vec![8u8, 9u8])] {
                        let mut file = overlay
                            .open_file(
                                path,
                                ClassicOpenMode {
                                    read: true,
                                    write: true,
                                    creation: 2,
                                },
                            )?
                            .expect("created");
                        file.write(0, &bytes)?;
                        file.close();
                    }
                    Ok(())
                },
                false,
            )
            .unwrap();
        assert_eq!(save.game, vec![7]);
        assert_eq!(save.level, vec![8, 9]);
        overlay
            .restore(&save, &identity(), |game, level| {
                for path in [game, level] {
                    let mut file = overlay
                        .open_file(
                            path,
                            ClassicOpenMode {
                                read: true,
                                write: false,
                                creation: 3,
                            },
                        )?
                        .expect("opened");
                    assert!(!file.read(0, 8)?.is_empty());
                    file.close();
                }
                Ok(())
            })
            .unwrap();
        // Read-only restore rejects writers.
        assert!(overlay
            .restore(&save, &identity(), |game, _| {
                assert!(overlay
                    .open_file(
                        game,
                        ClassicOpenMode {
                            read: true,
                            write: true,
                            creation: 3,
                        }
                    )
                    .is_err());
                for path in [game] {
                    let mut file = overlay
                        .open_file(
                            path,
                            ClassicOpenMode {
                                read: true,
                                write: false,
                                creation: 3,
                            },
                        )?
                        .expect("opened");
                    file.read(0, 8)?;
                    file.close();
                }
                let mut level = overlay
                    .open_file(
                        "__qts_original_save/level.sav",
                        ClassicOpenMode {
                            read: true,
                            write: false,
                            creation: 3,
                        },
                    )?
                    .expect("opened");
                level.read(0, 8)?;
                level.close();
                Ok(())
            })
            .is_ok());
    }

    #[test]
    fn travel_levels_round_trip() {
        let overlay = ClassicOriginalSaveFiles::new(None);
        let bytes = overlay
            .capture_travel_level(|level| {
                let mut file = overlay
                    .open_file(
                        level,
                        ClassicOpenMode {
                            read: true,
                            write: true,
                            creation: 2,
                        },
                    )?
                    .expect("created");
                file.write(0, &[1, 2, 3])?;
                file.close();
                Ok(())
            })
            .unwrap();
        assert_eq!(bytes, vec![1, 2, 3]);
        let size = overlay
            .with_travel_level(&bytes, |level| {
                let mut file = overlay
                    .open_file(
                        level,
                        ClassicOpenMode {
                            read: true,
                            write: false,
                            creation: 3,
                        },
                    )?
                    .expect("opened");
                let size = file.size()?;
                file.read(0, size)?;
                file.close();
                Ok(size)
            })
            .unwrap();
        assert_eq!(size, 3);
    }
}
