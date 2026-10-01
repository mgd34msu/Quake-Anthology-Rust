//! Quake III QVM server-game guest runtime.
//!
//! Port of donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/q3/guest-runtime.ts`
//! (`Q3QvmServerGame`, `savedQ3GuestClients`, `Q3GuestOutput`,
//! `Q3GuestRuntimeOptions`, map-transition types).
//!
//! Sync port: the donor is async throughout, but the Rust QVM port is
//! fully synchronous, so every method is sync and [`Q3GuestOutput`] is a
//! sync trait. There is no async runtime.
//!
//! Botlib is not wired: `attachBots`/`Q3GuestBotPreparation` need the
//! `Q3GuestBots` engine, whose `BotLibraryHost` surface (elementary
//! actions, chat, characters, goals, weapon weights, AAS library memory
//! and save-state from donor `src/bots/behavior/q3/library.ts`) has no
//! `qa-bots` home — the Rust `BotLibrary` only covers
//! setup/load-map/start-frame/shutdown. This module therefore implements
//! the donor's no-botlib configuration faithfully: `initialize` requires
//! `bot_enable` 0, checkpoints capture `bots: null` with empty bot-client
//! tables, restore rejects non-empty bot state, botlib traps fall into the
//! donor's `disabledBot` arm, and `is_bot` is always false. Map-carried
//! bot flags still round-trip as data. The bot lane adds `guest-bots.rs`
//! plus `attach_bots` without changing this file's protocol.
//!
//! Two universe bridges apply. The syscall dispatchers (`cvar_syscall`,
//! `file_syscall`, `server_game_syscall`) live in the `client_state`
//! universe (`HostCall`/`SyscallMemory`), while `QvmGame` hosts
//! `game_data` calls, so each trap copies the guest allocation
//! out-and-back (padding to a power of two when the image length is not
//! one; valid guest pointers are unaffected). The common traps
//! (`G_PRINT`/`G_ERROR`/`G_MILLISECONDS`/`G_ARGC`/`G_ARGV`/
//! `G_SEND_CONSOLE_COMMAND`/`G_REAL_TIME`) and the disabled-bot arm are
//! mirrored directly onto `game_data` calls because `qvm_common_syscall`
//! borrows the interpreter universe, which cannot be constructed here;
//! behavior matches `common-syscalls.ts` arm for arm.
//!
//! `Q3ApplicationPlayer`/`Q3ApplicationAdmission` are mirrored here (the
//! network lane owns donor `src/app/bootstrap/network/q3-types.ts`);
//! guest-movement imports them from this module.
//!
//! Client-session binding (`validateClient`) cannot compare identity
//! sessions: `ClientId` exposes no session accessor and the guest holds
//! no `IdentityOwner`. The port enforces slot range plus slot/client-slot
//! consistency; foreign clients cannot arrive structurally because slots
//! are admitted through the session's own `connect`/`reconnect` and
//! restore paths.
//!
//! Trap output is reentrancy-safe: trap handlers record [`HostIntent`]s
//! and the host drains them after releasing the state borrow, so output
//! callbacks may call back into the runtime. All other user callbacks
//! run with no borrow held.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

use qa_content::contract::ResourceProvenance;
use qa_content::mounts::MountedContent;
use qa_core::identity::{ActorId, ClientId, SavedActorId};
use qa_core::math::Vec3;
use qa_guest::error::GuestError;
use qa_guest::qvm::client_state::{
    AbiProfile as ClientAbi, CallKind as ClientKind, HostCall as ClientCall, QvmRole as ClientRole, SyscallMemory,
    WireUserCommand as ClientWireCommand,
};
use qa_guest::qvm::common_syscalls::QvmCalendar;
use qa_guest::qvm::cvar_syscalls::{cvar_syscall, CvarHost, CvarValue, CvarVmBinding};
use qa_guest::qvm::entity_tokens::QvmEntityTokens;
use qa_guest::qvm::file_syscalls::{
    file_syscall, FileMounts, HandleCheckpoint, OpenedFile, QvmFiles, ReadHandleCheckpoint, SeekOrigin, WritableFile,
    WritableStore, WriteHandleCheckpoint, WriteMode,
};
use qa_guest::qvm::game::QvmGame;
use qa_guest::qvm::game_data::{
    AbiProfile as GameAbi, CallKind as GameKind, ProfileValue, QvmArtifact, QvmCheckpoint, QvmFunctionCall,
    QvmGameDataState, QvmGameImport, QvmHostCall as GameCall, QvmHostFn, QvmHostState, QvmRole as GameRole,
};
use qa_guest::qvm::game_input::{
    QvmClientIdentity, QvmInputBinding, QvmInputDefinition, QvmInputServices, QvmInputSource,
};
use qa_guest::qvm::legacy_bot_abi::{
    BOTLIB_AAS_INITIALIZED, BOTLIB_LOAD_MAP, BOTLIB_SETUP, BOTLIB_SHUTDOWN, BOTLIB_START_FRAME, BOTLIB_TEST,
    BOTLIB_UPDATENTITY,
};
use qa_guest::qvm::server_game_syscalls::{server_game_syscall, ServerGameHost};
use qa_platform::files::writable::{UserFileStore, WritableBinaryFile, WritableFileMode};
use qa_world::movement::types::ArsenalState;
use qa_world::save::records::{read_saved_actor, write_saved_actor};
use qa_world::save::value::{
    arr, boolean, decode_checkpoint_value, encode_checkpoint_value, int, obj, str as save_str, SaveJson, SaveReader,
};
use qa_world::WorldError;

use super::guest_records::{Q3GuestRecordHost, Q3GuestRecords};
use super::guest_spatial::{Q3GuestClipFactory, Q3GuestSpatial};
use super::server_state::{Q3ServerState, Q3StoredUserCommand};

/// Host checkpoint format tag (donor `q3:qagame-host`).
const HOST_FORMAT: &str = "q3:qagame-host";

/// Host checkpoint version (donor `1`).
const HOST_VERSION: i64 = 1;

/// `G_ARGC` trap (donor `src/compat/qvm/abi.ts`; absent from the Rust
/// trap tables).
const G_ARGC: i32 = 8;
/// `G_ARGV` trap (donor `src/compat/qvm/abi.ts`).
const G_ARGV: i32 = 9;
/// `G_SEND_CONSOLE_COMMAND` trap (donor `src/compat/qvm/abi.ts`).
const G_SEND_CONSOLE_COMMAND: i32 = 14;
/// `G_REAL_TIME` trap (donor `src/compat/qvm/abi.ts`).
const G_REAL_TIME: i32 = 41;

/// Maximum `G_ERROR` text (donor truncates to 4095).
const MAX_ERROR_CHARS: usize = 4095;

/// `G_REAL_TIME` calendar payload size (9 words).
const CALENDAR_BYTES: usize = 36;

/// `qagame` application player: network-lane mirror (canonical home:
/// donor `src/app/bootstrap/network/q3-types.ts`).
/// `admission` is guest-private bookkeeping, not donor state: the donor
/// compares players by object identity, so the admission ordinal makes a
/// stale clone unequal to a re-admitted player under `PartialEq`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3ApplicationPlayer {
    /// Connected client.
    pub client: ClientId,
    /// Player actor.
    pub actor: ActorId,
    /// Source entity slot.
    pub source_entity: i32,
    /// Admission ordinal.
    admission: u64,
}

/// `qagame` admission decision: network-lane mirror (canonical home:
/// donor `src/app/bootstrap/network/q3-types.ts`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q3ApplicationAdmission {
    /// Admitted with a player binding.
    Accepted {
        /// Admitted player.
        player: Q3ApplicationPlayer,
    },
    /// Rejected with a reason.
    Rejected {
        /// Rejection reason.
        reason: String,
    },
}

/// Guest output operations behind the running game (sync port of the
/// donor's `Q3GuestOutput`).
pub trait Q3GuestOutput {
    /// Drop a client with a reason.
    fn drop_client(&self, slot: i32, reason: &str);
    /// Send a server command to a slot (`-1` broadcasts).
    fn send_server_command(&self, slot: i32, text: &str);
    /// Publish a configstring.
    fn configstring(&self, index: i32, value: &str);
}

/// Execute-now console callback.
pub type Q3GuestConsoleExecutor = Rc<dyn Fn(Option<&str>)>;
/// Console text callback.
pub type Q3GuestConsoleWriter = Rc<dyn Fn(&str)>;
/// Millisecond clock callback.
pub type Q3GuestMilliseconds = Rc<dyn Fn() -> i32>;
/// Real-time callback.
pub type Q3GuestRealTime = Rc<dyn Fn(Option<&mut dyn FnMut(QvmCalendar)>) -> i32>;
/// Actor callback.
pub type Q3GuestActorCallback = Rc<dyn Fn(&ActorId)>;
/// Thunk callback.
pub type Q3GuestThunk = Rc<dyn Fn()>;
/// Selected-game arsenal projection.
pub type Q3GuestArsenalProjection = Rc<dyn Fn(&ActorId) -> ArsenalState>;
/// Client-changed callback.
pub type Q3GuestClientChanged = Rc<dyn Fn(Q3ClientChangedKind, &ActorId)>;
/// Bot user-command callback.
pub type Q3GuestBotCommand = Rc<dyn Fn(&ActorId, &Q3StoredUserCommand)>;
/// Saved-client resolver.
pub type Q3GuestClientResolver = Rc<dyn Fn(&Q3SavedGuestClientId) -> ClientId>;

/// Console commands behind `G_SEND_CONSOLE_COMMAND` (donor
/// `ConsoleCommands`, sync port).
pub struct Q3GuestConsoleCommands {
    /// Execute console text now (`None` runs the buffer).
    pub execute_now: Q3GuestConsoleExecutor,
    /// Insert console text at the front.
    pub insert: Q3GuestConsoleWriter,
    /// Append console text.
    pub append: Q3GuestConsoleWriter,
}

/// Common trap services (donor `Pick<QvmCommonServices,
/// 'milliseconds' | 'realTime' | 'commands'>` for `qagame`).
pub struct Q3GuestCommon {
    /// Milliseconds since start.
    pub milliseconds: Q3GuestMilliseconds,
    /// Real time with an optional calendar sink.
    pub real_time: Q3GuestRealTime,
    /// Console commands.
    pub commands: Q3GuestConsoleCommands,
}

/// Client-changed notification kind (donor `'admitted' | 'userinfo'`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3ClientChangedKind {
    /// Client admitted.
    Admitted,
    /// Client userinfo changed.
    Userinfo,
}

/// Guest runtime construction options (donor `Q3GuestRuntimeOptions`).
pub struct Q3GuestRuntimeOptions {
    /// qagame artifact.
    pub artifact: QvmArtifact,
    /// Engine-owned server storage.
    pub state: Q3ServerState,
    /// Session host behind guest records.
    pub records: Q3GuestRecordHost,
    /// Mounted content for guest file reads.
    pub mounts: MountedContent,
    /// Writable store for guest file writes.
    pub writable: UserFileStore,
    /// Maximum clients (`1..=64`).
    pub max_clients: i32,
    /// Dedicated server flag (`None` means dedicated).
    pub dedicated: Option<bool>,
    /// Random seed for `GAME_INIT`.
    pub seed: i32,
    /// Entity text for the token stream.
    pub entity_text: String,
    /// Common trap services.
    pub common: Q3GuestCommon,
    /// Millisecond clock.
    pub now: Q3GuestMilliseconds,
    /// Assert the runtime is current.
    pub assert_current: Q3GuestThunk,
    /// Temporary clip-model factory (sibling seam: donor
    /// `createBoxModel`/`createCapsuleModel` have no Rust home).
    pub clip: Q3GuestClipFactory,
    /// Before-disconnect notification.
    pub before_disconnect: Option<Q3GuestActorCallback>,
    /// Before-retire notification.
    pub before_retire: Option<Q3GuestThunk>,
    /// Source-restored notification.
    pub source_restored: Option<Q3GuestThunk>,
    /// Selected-game arsenal projection.
    pub arsenal: Option<Q3GuestArsenalProjection>,
    /// Client-changed notification.
    pub client_changed: Option<Q3GuestClientChanged>,
    /// Bot user-command notification (unreachable until the bot lane
    /// lands; kept for interface parity).
    pub bot_command: Option<Q3GuestBotCommand>,
}

/// Saved guest client identity (donor `Q3SavedGuestClientId`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3SavedGuestClientId {
    /// Client slot.
    pub slot: u32,
    /// Slot generation.
    pub generation: u32,
}

/// Guest map client row (donor `Q3GuestMapClient`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3GuestMapClient {
    /// Carried client.
    pub client: ClientId,
    /// Carried userinfo.
    pub userinfo: String,
    /// Whether the client is a bot.
    pub bot: bool,
}

/// Guest map transition (donor `Q3GuestMapTransition`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3GuestMapTransition {
    /// Captured server state (a superset of the donor's cvar-only
    /// capture: the registry snapshot has no standalone Rust home, so
    /// the full server save image carries it).
    pub cvars: SaveJson,
    /// Carried clients.
    pub clients: Vec<Q3GuestMapClient>,
}

/// Guest initialization mode (donor `Q3GuestInitialization`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q3GuestInitialization {
    /// Fresh map.
    New,
    /// Map change with carried clients.
    MapChange(Vec<Q3GuestMapClient>),
    /// Map restart with carried clients.
    MapRestart(Vec<Q3GuestMapClient>),
}

/// Saved client phase (donor `'connected' | 'active'`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3SavedClientPhase {
    /// Connected.
    Connected,
    /// Active.
    Active,
}

/// Saved guest client row (donor `savedQ3GuestClients` entry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3SavedGuestClient {
    /// Source entity slot.
    pub source_entity: i32,
    /// Saved client identity.
    pub client: Q3SavedGuestClientId,
    /// Saved actor.
    pub actor: SavedActorId,
    /// Saved phase.
    pub phase: Q3SavedClientPhase,
}

fn read_saved_phase(reader: &SaveReader) -> Result<Q3SavedClientPhase, WorldError> {
    match reader.choice_str(&["connected", "active"])?.as_str() {
        "connected" => Ok(Q3SavedClientPhase::Connected),
        _ => Ok(Q3SavedClientPhase::Active),
    }
}

fn read_guest_clients(reader: &SaveReader) -> Result<Vec<Q3SavedGuestClient>, WorldError> {
    reader.list(|entry| {
        let client = entry.field("client");
        Ok(Q3SavedGuestClient {
            source_entity: entry.field("sourceEntity").integer(0)? as i32,
            client: Q3SavedGuestClientId {
                slot: client.field("slot").integer(0)? as u32,
                generation: client.field("generation").integer(0)? as u32,
            },
            actor: read_saved_actor(entry.field("actor"))?,
            phase: read_saved_phase(&entry.field("phase"))?,
        })
    })
}

/// Read saved guest clients from a module checkpoint without restoring
/// (donor `savedQ3GuestClients`).
pub fn saved_q3_guest_clients(checkpoint: &QvmCheckpoint) -> Result<Vec<Q3SavedGuestClient>, GuestError> {
    if checkpoint.host_state.format != HOST_FORMAT {
        return Err(GuestError::invalid("Unsupported Q3 guest host checkpoint format"));
    }
    let ProfileValue::Bytes(bytes) = &checkpoint.host_state.bytes else {
        return Err(GuestError::invalid("Unsupported Q3 guest host checkpoint format"));
    };
    let value = decode_checkpoint_value(bytes).map_err(|error| GuestError::invalid(error.to_string()))?;
    let reader = SaveReader::at(&value, "q3.guest.host");
    let fail = |message: &str| GuestError::invalid(format!("q3.guest.host: {message}"));
    reader
        .field("version")
        .literal_i64(HOST_VERSION)
        .map_err(|_| fail("version mismatch"))?;
    let maximum = reader
        .field("maxClients")
        .integer(1)
        .map_err(|_| fail("invalid guest client capacity"))?;
    if maximum > 64 {
        return Err(fail("invalid guest client capacity"));
    }
    let clients = read_guest_clients(&reader.field("clients")).map_err(|error| fail(&error.to_string()))?;
    let mut slots = HashSet::new();
    for entry in &clients {
        if i64::from(entry.source_entity) >= maximum
            || entry.source_entity != entry.client.slot as i32
            || !slots.insert(entry.source_entity)
        {
            return Err(fail("invalid or duplicate source client slot"));
        }
    }
    Ok(clients)
}

/// Deferred trap effect, drained after the state borrow releases so
/// output callbacks may reenter the runtime.
enum HostIntent {
    /// Drop a client with a reason.
    DropClient {
        /// Client slot.
        slot: i32,
        /// Drop reason.
        reason: String,
    },
    /// Send a server command.
    SendServerCommand {
        /// Client slot (`-1` broadcasts).
        slot: i32,
        /// Command text.
        text: String,
    },
    /// Publish a configstring.
    Configstring {
        /// Configstring index.
        index: i32,
        /// Configstring value.
        value: String,
    },
}

/// Client phase (donor `ClientEntry['phase']`).
#[derive(Debug, Clone, PartialEq, Eq)]
enum ClientPhase {
    /// Connecting.
    Connecting,
    /// Connected.
    Connected,
    /// Active.
    Active,
    /// Dropping with a reason.
    Dropping {
        /// Drop reason.
        reason: String,
    },
}

/// Admitted client entry (donor `ClientEntry`; the donor's object
/// identity becomes an admission ordinal).
struct ClientEntry {
    /// Admitted player.
    player: Q3ApplicationPlayer,
    /// Admission ordinal (distinguishes re-admissions per slot).
    ordinal: u64,
    /// Whether disconnect was notified.
    disconnect_notified: bool,
    /// Client phase.
    phase: ClientPhase,
}

/// Input-retirement phase (donor `InputRetirement['phase']`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetirementPhase {
    /// Pending.
    Pending,
    /// Disconnecting.
    Disconnecting,
    /// Complete.
    Complete,
}

/// Input retirement (donor `InputRetirement`).
struct InputRetirement {
    /// Retiring identity.
    identity: QvmClientIdentity,
    /// Admission ordinal, when a client entry existed.
    ordinal: Option<u64>,
    /// Retirement phase.
    phase: RetirementPhase,
}

/// Lifecycle (donor `Lifecycle`).
enum Lifecycle {
    /// Created.
    Created,
    /// Restoring.
    Restoring,
    /// Restored.
    Restored,
    /// Retired.
    Retired,
    /// Initializing with output.
    Initializing {
        /// Guest output.
        output: Rc<dyn Q3GuestOutput>,
    },
    /// Running with output.
    Running {
        /// Guest output.
        output: Rc<dyn Q3GuestOutput>,
    },
    /// Shutting down with output.
    ShuttingDown {
        /// Guest output.
        output: Rc<dyn Q3GuestOutput>,
    },
}

/// `FileMounts` bridge over mounted content.
struct MountBridge {
    /// Mounted content.
    mounts: MountedContent,
}

impl FileMounts for MountBridge {
    fn open(&mut self, path: &str) -> Option<OpenedFile> {
        let found = self.mounts.open(path, |_| true).ok()??;
        Some(OpenedFile {
            bytes: found.bytes,
            pk3: matches!(found.reference.provenance, ResourceProvenance::Archive { .. }),
        })
    }

    fn list_files(&mut self, path: &str, extension: &str) -> Vec<String> {
        self.mounts.list_files(path, extension).unwrap_or_default()
    }
}

/// `WritableFile` bridge over a platform binary file.
struct WritableBridge {
    /// Platform file.
    file: WritableBinaryFile,
    /// File path.
    path: String,
    /// Open mode.
    mode: WriteMode,
}

impl WritableFile for WritableBridge {
    fn write(&mut self, bytes: &[u8]) -> usize {
        self.file.write(bytes).unwrap_or(0)
    }

    fn seek(&mut self, offset: i32, origin: SeekOrigin) -> i32 {
        let _ = self.file.seek(
            i64::from(offset),
            match origin {
                SeekOrigin::Current => qa_platform::files::writable::WritableSeekOrigin::Current,
                SeekOrigin::End => qa_platform::files::writable::WritableSeekOrigin::End,
                SeekOrigin::Set => qa_platform::files::writable::WritableSeekOrigin::Set,
            },
        );
        0
    }

    fn position(&self) -> usize {
        self.file.position().unwrap_or(0) as usize
    }

    fn path(&self) -> &str {
        &self.path
    }

    fn mode(&self) -> WriteMode {
        self.mode
    }
}

/// `WritableStore` bridge over the user file store.
struct WritableStoreBridge {
    /// User file store.
    store: UserFileStore,
}

fn writable_mode(mode: WriteMode) -> WritableFileMode {
    match mode {
        WriteMode::Write => WritableFileMode::Write,
        WriteMode::Append => WritableFileMode::Append,
        WriteMode::AppendSync => WritableFileMode::AppendSync,
    }
}

impl WritableStore for WritableStoreBridge {
    fn open_file(&mut self, path: &str, mode: WriteMode) -> Option<Box<dyn WritableFile>> {
        let file = self.store.open(path, writable_mode(mode), |_| {}).ok()??;
        Some(Box::new(WritableBridge {
            file,
            path: path.to_string(),
            mode,
        }))
    }

    fn resume_file(&mut self, path: &str, mode: WriteMode, position: usize) -> Box<dyn WritableFile> {
        let checkpoint = qa_platform::files::writable::WritableFileCheckpoint {
            path: path.to_string(),
            mode: writable_mode(mode),
            position: position as u64,
        };
        let file = self
            .store
            .resume(&checkpoint, |_| {})
            .unwrap_or_else(|_| panic!("Q3 guest resume must reopen its writable file {path}"));
        Box::new(WritableBridge {
            file,
            path: path.to_string(),
            mode,
        })
    }
}

fn write_mode_name(mode: WriteMode) -> &'static str {
    match mode {
        WriteMode::Write => "write",
        WriteMode::Append => "append",
        WriteMode::AppendSync => "append-sync",
    }
}

fn parse_write_mode(name: &str) -> Result<WriteMode, WorldError> {
    match name {
        "write" => Ok(WriteMode::Write),
        "append" => Ok(WriteMode::Append),
        "append-sync" => Ok(WriteMode::AppendSync),
        _ => Err(WorldError::BadSave(format!(
            "q3.guest.host: unknown guest file write mode {name}"
        ))),
    }
}

fn encode_files(checkpoint: &qa_guest::qvm::file_syscalls::FilesCheckpoint) -> SaveJson {
    arr(checkpoint
        .handles
        .iter()
        .map(|(slot, handle)| match handle {
            HandleCheckpoint::Read(read) => obj(vec![
                ("slot", int(i64::from(*slot))),
                ("kind", save_str("read")),
                ("bytes", save_str(&qa_world::save::value::base64_encode(&read.bytes))),
                ("position", int(read.position as i64)),
                ("pk3", boolean(read.pk3)),
            ]),
            HandleCheckpoint::Write(write) => obj(vec![
                ("slot", int(i64::from(*slot))),
                ("kind", save_str("write")),
                ("path", save_str(&write.path)),
                ("mode", save_str(write_mode_name(write.mode))),
                ("position", int(write.position as i64)),
            ]),
        })
        .collect())
}

fn decode_files(reader: &SaveReader) -> Result<qa_guest::qvm::file_syscalls::FilesCheckpoint, WorldError> {
    let handles = reader.list(|entry| {
        let slot = entry.field("slot").integer(0)? as i32;
        let handle = match entry.field("kind").choice_str(&["read", "write"])?.as_str() {
            "read" => HandleCheckpoint::Read(ReadHandleCheckpoint {
                bytes: entry.field("bytes").bytes()?,
                position: entry.field("position").integer(0)? as usize,
                pk3: entry.field("pk3").boolean()?,
            }),
            _ => HandleCheckpoint::Write(WriteHandleCheckpoint {
                path: entry.field("path").string()?,
                mode: parse_write_mode(&entry.field("mode").string()?)?,
                position: entry.field("position").integer(0)? as usize,
            }),
        };
        Ok((slot, handle))
    })?;
    Ok(qa_guest::qvm::file_syscalls::FilesCheckpoint { handles })
}

/// Interior runtime state behind the host closure.
struct Inner {
    /// Guest game (shared handle).
    game: QvmGame,
    /// Engine-owned server storage.
    state: Rc<Q3ServerState>,
    /// Guest records.
    records: Rc<Q3GuestRecords>,
    /// Guest spatial operations.
    spatial: Rc<Q3GuestSpatial>,
    /// Exclusive spatial handle for trap dispatch (shares every handle
    /// with `spatial`; the struct carries no local state).
    spatial_ops: Q3GuestSpatial,
    /// Guest files.
    files: QvmFiles,
    /// Entity-token stream.
    tokens: QvmEntityTokens,
    /// Entity text (checkpoint literal).
    entity_text: String,
    /// Random seed for `GAME_INIT`.
    seed: i32,
    /// Maximum clients.
    max_clients: i32,
    /// Common trap services.
    common: Q3GuestCommon,
    /// Millisecond clock.
    now: Q3GuestMilliseconds,
    /// Assert-current callback.
    assert_current: Q3GuestThunk,
    /// Before-disconnect notification.
    before_disconnect: Option<Q3GuestActorCallback>,
    /// Before-retire notification.
    before_retire: Option<Q3GuestThunk>,
    /// Source-restored notification.
    pub source_restored: Option<Q3GuestThunk>,
    /// Selected-game arsenal projection.
    pub arsenal: Option<Q3GuestArsenalProjection>,
    /// Client-changed notification.
    pub client_changed: Option<Q3GuestClientChanged>,
    /// Lifecycle.
    lifecycle: Lifecycle,
    /// Active external operations.
    external_ops: i32,
    /// Current trap code, while dispatching.
    current_code: Option<i32>,
    /// Admitted clients by slot.
    clients: HashMap<i32, ClientEntry>,
    /// Next admission ordinal.
    next_ordinal: u64,
    /// Map-carried clients by slot.
    reconnecting: HashMap<i32, Q3GuestMapClient>,
    /// Input retirements by slot.
    retirements: HashMap<i32, InputRetirement>,
    /// Restore client resolver.
    restore_client: Option<Q3GuestClientResolver>,
    /// Input binding.
    input_binding: Option<QvmInputBinding>,
    /// Deferred trap effects.
    intents: Vec<HostIntent>,
    /// VM cvar bindings by handle.
    vm_cvars: HashMap<i32, String>,
    /// Next VM cvar handle.
    next_vm_handle: i32,
}

fn stored_to_wire(command: &Q3StoredUserCommand) -> ClientWireCommand {
    ClientWireCommand {
        server_time: command.server_time,
        angles: command.angles,
        buttons: command.buttons,
        weapon: command.weapon as u8,
        forwardmove: command.forwardmove as i8,
        rightmove: command.rightmove as i8,
        upmove: command.upmove as i8,
    }
}

fn default_stored_command() -> Q3StoredUserCommand {
    Q3StoredUserCommand {
        server_time: 0,
        angles: [0, 0, 0],
        forwardmove: 0,
        rightmove: 0,
        upmove: 0,
        buttons: 0,
        weapon: 0,
    }
}

impl CvarHost for Inner {
    fn bind_vm(&mut self, name: &str, default: &str, flags: i32) -> i32 {
        let handle = self.next_vm_handle;
        self.next_vm_handle += 1;
        self.vm_cvars.insert(handle, name.to_string());
        let _ = self.state.cvars.borrow_mut().register(name, default, flags as u32);
        handle
    }

    fn read_vm(&mut self, handle: i32) -> Option<CvarVmBinding> {
        let name = self.vm_cvars.get(&handle)?.clone();
        let snapshot = self.state.cvars.borrow().get(&name)?;
        Some(CvarVmBinding {
            modification_count: snapshot.modification_count as i32,
            value: snapshot.value,
            numeric_value: snapshot.numeric_value,
            integer_value: snapshot.integer_value,
        })
    }

    fn get(&mut self, name: &str) -> Option<CvarValue> {
        let snapshot = self.state.cvars.borrow().get(name)?;
        Some(CvarValue {
            value: snapshot.value,
            numeric_value: snapshot.numeric_value,
            integer_value: snapshot.integer_value,
        })
    }

    fn set(&mut self, name: &str, value: &str) {
        let _ = self.state.cvars.borrow_mut().set(name, value, false);
    }

    fn set_value(&mut self, name: &str, value: f32) {
        let _ = self.state.cvars.borrow_mut().set_value(name, f64::from(value));
    }

    fn reset(&mut self, name: &str) {
        let _ = self.state.cvars.borrow_mut().reset(name, false);
    }

    fn register(&mut self, name: &str, default: &str, flags: i32) {
        let _ = self.state.cvars.borrow_mut().register(name, default, flags as u32);
    }

    fn info_string(&mut self, flags: i32) -> String {
        self.state
            .cvars
            .borrow_mut()
            .info_string(flags as u32, None)
            .unwrap_or_default()
    }
}

impl ServerGameHost for Inner {
    fn abi_profile(&self) -> ClientAbi {
        match self.game.module.abi_profile() {
            GameAbi::Modern => ClientAbi::Modern,
            GameAbi::Legacy => ClientAbi::Legacy,
        }
    }

    fn max_clients(&self) -> i32 {
        self.max_clients
    }

    fn number_from_pointer(&mut self, word: i32) -> i32 {
        self.game
            .data
            .number_from_pointer(word)
            .map(|slot| slot as i32)
            .unwrap_or_else(|_| panic!("Q3 guest entity pointer {word} has no source slot"))
    }

    fn get_userinfo(&mut self, slot: i32) -> String {
        self.state.get_userinfo(slot).unwrap_or_default()
    }

    fn set_userinfo(&mut self, slot: i32, value: &str) {
        self.state.set_userinfo(slot, value);
    }

    fn get_user_command(&mut self, slot: i32) -> ClientWireCommand {
        stored_to_wire(&self.state.get_user_command(slot).unwrap_or_else(default_stored_command))
    }

    fn drop_client(&mut self, slot: i32, reason: &str) {
        self.intents.push(HostIntent::DropClient {
            slot,
            reason: reason.to_string(),
        });
    }

    fn send_server_command(&mut self, slot: i32, text: &str) {
        self.intents.push(HostIntent::SendServerCommand {
            slot,
            text: text.to_string(),
        });
    }

    fn config_get(&mut self, index: i32) -> String {
        self.state.configstring_get(index)
    }

    fn config_set(&mut self, index: i32, value: &str) {
        if self.state.configstring_get(index) == value {
            return;
        }
        self.state.configstring_set(index, value);
        self.intents.push(HostIntent::Configstring {
            index,
            value: value.to_string(),
        });
    }

    fn server_info(&mut self) -> String {
        self.state.server_info()
    }

    fn entity_token(&mut self) -> (String, bool) {
        self.tokens.entity_token().unwrap_or_default()
    }
}

impl qa_guest::qvm::server_game_syscalls::ServerSpatialHost for Inner {
    fn trace(
        &mut self,
        query: &qa_guest::qvm::server_game_syscalls::ServerTraceQuery,
    ) -> qa_guest::qvm::client_collision_syscalls::TraceRecord {
        qa_guest::qvm::server_game_syscalls::ServerSpatialHost::trace(&mut self.spatial_ops, query)
    }

    fn point_contents(&mut self, point: Vec3, pass_entity_num: i32) -> i32 {
        qa_guest::qvm::server_game_syscalls::ServerSpatialHost::point_contents(
            &mut self.spatial_ops,
            point,
            pass_entity_num,
        )
    }

    fn area_entities(&mut self, bounds: qa_guest::qvm::server_game_syscalls::Bounds, maximum: i32) -> Vec<i32> {
        qa_guest::qvm::server_game_syscalls::ServerSpatialHost::area_entities(&mut self.spatial_ops, bounds, maximum)
    }

    fn entity_contact(
        &mut self,
        bounds: qa_guest::qvm::server_game_syscalls::Bounds,
        slot: i32,
        capsule: bool,
    ) -> bool {
        qa_guest::qvm::server_game_syscalls::ServerSpatialHost::entity_contact(
            &mut self.spatial_ops,
            bounds,
            slot,
            capsule,
        )
    }

    fn set_brush_model(&mut self, slot: i32, name: &str) {
        qa_guest::qvm::server_game_syscalls::ServerSpatialHost::set_brush_model(&mut self.spatial_ops, slot, name);
    }

    fn adjust_area_portal_state(&mut self, slot: i32, open: bool) {
        qa_guest::qvm::server_game_syscalls::ServerSpatialHost::adjust_area_portal_state(
            &mut self.spatial_ops,
            slot,
            open,
        );
    }

    fn in_pvs(&mut self, first: Vec3, second: Vec3, ignore_portals: bool) -> bool {
        qa_guest::qvm::server_game_syscalls::ServerSpatialHost::in_pvs(
            &mut self.spatial_ops,
            first,
            second,
            ignore_portals,
        )
    }

    fn areas_connected(&mut self, first: i32, second: i32) -> bool {
        qa_guest::qvm::server_game_syscalls::ServerSpatialHost::areas_connected(&mut self.spatial_ops, first, second)
    }

    fn link(&mut self, slot: i32) {
        qa_guest::qvm::server_game_syscalls::ServerSpatialHost::link(&mut self.spatial_ops, slot);
    }

    fn unlink(&mut self, slot: i32) {
        qa_guest::qvm::server_game_syscalls::ServerSpatialHost::unlink(&mut self.spatial_ops, slot);
    }
}

/// Bridged trap: client-universe call plus writable memory plus the real
/// allocation length (the bridge pads to a power of two).
struct BridgedTrap {
    /// Client-universe call.
    call: ClientCall,
    /// Writable guest memory.
    memory: SyscallMemory,
    /// Real allocation length (prefix of the padded memory).
    real_len: usize,
}

fn bridge_trap(call: &GameCall) -> Result<BridgedTrap, GuestError> {
    let kind = match call.kind {
        GameKind::Engine => ClientKind::Engine,
        GameKind::Extension => ClientKind::Extension,
    };
    let role = match call.role {
        GameRole::Qagame => ClientRole::Qagame,
        GameRole::Cgame => ClientRole::Cgame,
        GameRole::Ui => ClientRole::Ui,
    };
    let abi = match call.abi_profile {
        GameAbi::Modern => ClientAbi::Modern,
        GameAbi::Legacy => ClientAbi::Legacy,
    };
    let real_len = call.guest.len();
    let mut bytes = call.guest.read_bytes(0, real_len)?;
    let padded = real_len.next_power_of_two().max(64);
    bytes.resize(padded, 0);
    Ok(BridgedTrap {
        call: ClientCall {
            kind,
            role,
            code: call.code,
            words: call.words.clone(),
            abi_profile: abi,
        },
        memory: SyscallMemory::from_bytes(bytes)?,
        real_len,
    })
}

fn unbridge_trap(call: &GameCall, bridged: &BridgedTrap) -> Result<(), GuestError> {
    call.guest
        .write_bytes(0, &bridged.memory.as_slice()[..bridged.real_len])
}

fn argv_of(call: &GameCall) -> Vec<String> {
    call.command_arguments.clone().unwrap_or_default()
}

impl Inner {
    /// Mirror the non-cvar common traps onto a `game_data` call.
    fn common_trap(&mut self, call: &GameCall) -> Result<Option<i32>, GuestError> {
        if call.kind != GameKind::Engine || call.role != GameRole::Qagame {
            return Ok(None);
        }
        match call.code {
            c if c == QvmGameImport::G_PRINT => {
                let text = call.guest.read_string(call.int(1)?)?;
                self.state.print(&text);
                Ok(Some(0))
            }
            c if c == QvmGameImport::G_ERROR => {
                let mut text = call.guest.read_string(call.int(1)?)?;
                if text.len() > MAX_ERROR_CHARS {
                    text.truncate(MAX_ERROR_CHARS);
                }
                Err(GuestError::callback(format!("QVM game error: {text}")))
            }
            c if c == QvmGameImport::G_MILLISECONDS => Ok(Some((self.common.milliseconds)())),
            c if c == G_ARGC => Ok(Some(argv_of(call).len() as i32)),
            c if c == G_ARGV => {
                let argv = argv_of(call);
                let index = call.int(1)?;
                let text = argv
                    .get(usize::try_from(index).unwrap_or(usize::MAX))
                    .cloned()
                    .unwrap_or_default();
                call.guest
                    .write_string(call.int(2)?, &text, usize::try_from(call.int(3)?).unwrap_or(0))?;
                Ok(Some(0))
            }
            c if c == G_SEND_CONSOLE_COMMAND => {
                let when = call.int(1)?;
                let pointer = call.int(2)?;
                match when {
                    0 => {
                        let text = if pointer == 0 {
                            None
                        } else {
                            Some(call.guest.read_string(pointer)?)
                        };
                        (self.common.commands.execute_now)(text.as_deref());
                        Ok(Some(0))
                    }
                    1 => {
                        let text = call.guest.read_string(pointer)?;
                        (self.common.commands.insert)(&text);
                        Ok(Some(0))
                    }
                    2 => {
                        let text = call.guest.read_string(pointer)?;
                        (self.common.commands.append)(&text);
                        Ok(Some(0))
                    }
                    _ => Err(GuestError::invalid("Cbuf_ExecuteText: bad exec_when")),
                }
            }
            c if c == G_REAL_TIME => {
                let pointer = call.int(1)?;
                if pointer == 0 {
                    return Ok(Some((self.common.real_time)(None)));
                }
                call.guest.span(pointer, CALENDAR_BYTES)?;
                let guest = call.guest.clone();
                let write = |calendar: QvmCalendar| -> Result<(), GuestError> {
                    let fields = [
                        calendar.second,
                        calendar.minute,
                        calendar.hour,
                        calendar.day,
                        calendar.month,
                        calendar.year,
                        calendar.weekday,
                        calendar.year_day,
                        calendar.is_dst,
                    ];
                    let base = guest
                        .pointer(pointer)
                        .ok_or_else(|| GuestError::invalid("QVM real-time calendar is outside its allocation"))?;
                    for (index, value) in fields.iter().enumerate() {
                        guest.write_i32(base + index * 4, *value)?;
                    }
                    Ok(())
                };
                let mut sink = |calendar: QvmCalendar| {
                    write(calendar).unwrap_or_else(|_| panic!("QVM calendar span was preflighted"))
                };
                Ok(Some((self.common.real_time)(Some(&mut sink))))
            }
            _ => Ok(None),
        }
    }

    /// Disabled-botlib fallback (donor `disabledBot`).
    fn disabled_bot(&self, call: &GameCall) -> Option<i32> {
        if call.kind != GameKind::Engine || call.role != GameRole::Qagame {
            return None;
        }
        match call.code {
            BOTLIB_SETUP | BOTLIB_AAS_INITIALIZED | BOTLIB_TEST => Some(0),
            BOTLIB_SHUTDOWN => {
                self.state
                    .print("BotLibShutdown: bot library used before being setup\n");
                Some(1)
            }
            BOTLIB_START_FRAME => {
                self.state.print("BotStartFrame: bot library used before being setup\n");
                Some(1)
            }
            BOTLIB_LOAD_MAP => {
                self.state.print("BotLoadMap: bot library used before being setup\n");
                Some(1)
            }
            BOTLIB_UPDATENTITY => {
                self.state
                    .print("BotUpdateEntity: bot library used before being setup\n");
                Some(1)
            }
            _ => None,
        }
    }

    /// Dispatch one trap; records intents for the host to drain.
    fn dispatch(&mut self, call: &GameCall) -> Result<Option<i32>, GuestError> {
        if let Some(value) = self.common_trap(call)? {
            return Ok(Some(value));
        }
        if call.kind == GameKind::Engine {
            let mut bridged = bridge_trap(call)?;
            let result = cvar_syscall(&bridged.call, &mut bridged.memory, self)?.or(file_syscall(
                &bridged.call,
                &mut bridged.memory,
                &mut self.files,
            )?
            .or(server_game_syscall(&bridged.call, &mut bridged.memory, self)?));
            unbridge_trap(call, &bridged)?;
            if result.is_some() {
                return Ok(result);
            }
        }
        if let Some(value) = self.disabled_bot(call) {
            return Ok(Some(value));
        }
        Ok(None)
    }
}

/// `QvmInputSource` bridge into the runtime.
struct GuestInputSource {
    /// Guest game.
    game: QvmGame,
    /// Input declaration.
    definition: QvmInputDefinition,
    /// Guest records.
    records: Rc<Q3GuestRecords>,
    /// Runtime state.
    inner: Weak<RefCell<Inner>>,
}

impl GuestInputSource {
    /// Run a body against runtime state (the binding never outlives
    /// the runtime).
    fn with_inner<R>(&self, run: impl FnOnce(&mut Inner) -> R) -> R {
        let upgraded = self
            .inner
            .upgrade()
            .unwrap_or_else(|| panic!("QVM input outlives its guest runtime"));
        let mut guard = upgraded.borrow_mut();
        run(&mut guard)
    }
}

impl QvmInputSource for GuestInputSource {
    fn game(&self) -> &QvmGame {
        &self.game
    }

    fn definition(&self) -> &QvmInputDefinition {
        &self.definition
    }

    fn retiring(&self, slot: usize, identity: &QvmClientIdentity) {
        self.with_inner(|locked| {
            Q3QvmServerGame::retire_input_inner(locked, &self.records, slot as i32, identity);
        });
    }

    fn retired(&self, slot: usize) -> bool {
        self.records.is_input_retired(slot as i32)
    }

    fn disconnect(
        &self,
        slot: usize,
        identity: &QvmClientIdentity,
        _call: &mut QvmFunctionCall,
    ) -> Result<(), GuestError> {
        self.with_inner(|locked| {
            Q3QvmServerGame::disconnect_input_inner(locked, &self.records, &self.game, slot as i32, identity)
        })
    }

    fn movement(
        &self,
        slot: usize,
        call: &mut QvmFunctionCall,
        run: &mut dyn FnMut(&mut QvmFunctionCall) -> Result<i32, GuestError>,
    ) -> Result<i32, GuestError> {
        let mut result: Option<Result<i32, GuestError>> = None;
        let code = self.records.with_input_motion(slot as i32, || match run(call) {
            Ok(value) => {
                result = Some(Ok(value));
                value
            }
            Err(error) => {
                result = Some(Err(error));
                0
            }
        });
        result.unwrap_or(Ok(code))
    }
}

/// The guest owns all game-private state; application operations own
/// external serialization (donor `Q3QvmServerGame`, sync port without
/// botlib; see the module docs).
pub struct Q3QvmServerGame {
    /// Engine-owned server storage.
    pub state: Rc<Q3ServerState>,
    /// Guest game.
    pub game: QvmGame,
    /// Guest records.
    pub records: Rc<Q3GuestRecords>,
    /// Guest spatial operations.
    pub spatial: Rc<Q3GuestSpatial>,
    /// Interior state.
    inner: Rc<RefCell<Inner>>,
}

fn combine_errors(message: &str, errors: &[String]) -> GuestError {
    GuestError::callback(format!("{message}: {}", errors.join("; ")))
}

impl Q3QvmServerGame {
    /// Build a guest over an artifact and its session owners.
    pub fn new(options: Q3GuestRuntimeOptions) -> Result<Self, GuestError> {
        if options.max_clients < 1 || options.max_clients > 64 {
            panic!("Q3 guest requires 1..64 clients");
        }
        let state = Rc::new(options.state);
        let _ = state.cvars.borrow_mut().register("bot_enable", "1", 0);
        let _ = state
            .cvars
            .borrow_mut()
            .set("sv_maxclients", &options.max_clients.to_string(), true);
        let _ = state.cvars.borrow_mut().set(
            "dedicated",
            if options.dedicated == Some(false) { "0" } else { "1" },
            true,
        );
        let print_state = Rc::clone(&state);
        let mut files = QvmFiles::new(
            Box::new(MountBridge { mounts: options.mounts }),
            Some(Box::new(WritableStoreBridge {
                store: options.writable,
            })),
            move |text| print_state.print(text),
        );
        let host_slot: Rc<RefCell<Option<QvmHostFn>>> = Rc::new(RefCell::new(None));
        let dispatch_slot = Rc::clone(&host_slot);
        let game = QvmGame::new(
            options.artifact,
            Rc::new(move |call| dispatch_slot.borrow().as_ref().map_or(Ok(None), |host| host(call))),
        )?;
        let entity_text = options.entity_text;
        let built = (|| -> Result<(Rc<Q3GuestRecords>, Rc<Q3GuestSpatial>, QvmEntityTokens), GuestError> {
            game.data.set_client_count(options.max_clients as usize)?;
            let records = Q3GuestRecords::open(game.data.clone(), options.records);
            let spatial = Rc::new(Q3GuestSpatial::new(
                Rc::clone(&records),
                Rc::clone(&records.host.scene),
                Rc::clone(&state.cvars),
                Rc::clone(&options.clip),
            ));
            let tokens = QvmEntityTokens::new(entity_text.as_bytes().to_vec());
            Ok((records, spatial, tokens))
        })();
        let (records, spatial, tokens) = match built {
            Ok(built) => built,
            Err(error) => {
                files.close_all();
                game.retire();
                return Err(error);
            }
        };
        let spatial_ops = Q3GuestSpatial::new(
            Rc::clone(&records),
            Rc::clone(&records.host.scene),
            Rc::clone(&state.cvars),
            options.clip,
        );
        let inner = Rc::new(RefCell::new(Inner {
            spatial_ops,
            game: game.clone(),
            state: Rc::clone(&state),
            records: Rc::clone(&records),
            spatial: Rc::clone(&spatial),
            files,
            tokens,
            entity_text,
            seed: options.seed,
            max_clients: options.max_clients,
            common: options.common,
            now: options.now,
            assert_current: options.assert_current,
            before_disconnect: options.before_disconnect,
            before_retire: options.before_retire,
            source_restored: options.source_restored,
            arsenal: options.arsenal,
            client_changed: options.client_changed,
            lifecycle: Lifecycle::Created,
            external_ops: 0,
            current_code: None,
            clients: HashMap::new(),
            next_ordinal: 0,
            reconnecting: HashMap::new(),
            retirements: HashMap::new(),
            restore_client: None,
            input_binding: None,
            intents: Vec::new(),
            vm_cvars: HashMap::new(),
            next_vm_handle: 0,
        }));
        let host_inner = Rc::clone(&inner);
        *host_slot.borrow_mut() = Some(Rc::new(move |call| Self::host(&host_inner, call)));
        Ok(Self {
            state,
            game,
            records,
            spatial,
            inner,
        })
    }
}

impl Q3QvmServerGame {
    /// Assert the guest is live and current (donor `current`).
    fn current_inner(locked: &Inner) {
        if matches!(locked.lifecycle, Lifecycle::Retired) {
            panic!("Q3 guest is retired");
        }
        (locked.assert_current)();
    }

    /// Assert the guest is running outside a trap (donor `running`).
    fn running_inner(locked: &Inner) {
        Self::current_inner(locked);
        if !matches!(locked.lifecycle, Lifecycle::Running { .. }) {
            panic!("Q3 guest is not running");
        }
        if locked.current_code.is_some() {
            panic!("Q3 guest already has an active external operation");
        }
    }

    /// Attached output (donor `output`).
    fn output_inner(locked: &Inner) -> Rc<dyn Q3GuestOutput> {
        match &locked.lifecycle {
            Lifecycle::Initializing { output } | Lifecycle::Running { output } | Lifecycle::ShuttingDown { output } => {
                Rc::clone(output)
            }
            _ => panic!("Q3 guest output is unavailable"),
        }
    }

    /// Dispatch a trap through services; drain deferred intents after
    /// the borrow releases.
    fn host(inner: &Rc<RefCell<Inner>>, call: &GameCall) -> Result<Option<i32>, GuestError> {
        let result = {
            let mut locked = inner.borrow_mut();
            Self::current_inner(&locked);
            let previous = locked.current_code;
            locked.current_code = Some(call.code);
            let result = locked.dispatch(call);
            locked.current_code = previous;
            Self::current_inner(&locked);
            result
        };
        match result {
            Err(error) => {
                let _ = Self::drain_intents(inner);
                Err(error)
            }
            Ok(value) => {
                Self::drain_intents(inner)?;
                Ok(value)
            }
        }
    }

    /// Drain deferred trap intents with no borrow held.
    fn drain_intents(inner: &Rc<RefCell<Inner>>) -> Result<(), GuestError> {
        let intents = std::mem::take(&mut inner.borrow_mut().intents);
        for intent in intents {
            match intent {
                HostIntent::DropClient { slot, reason } => Self::drop_inner(inner, slot, &reason)?,
                HostIntent::SendServerCommand { slot, text } => {
                    let output = {
                        let locked = inner.borrow();
                        Self::output_inner(&locked)
                    };
                    output.send_server_command(slot, &text);
                }
                HostIntent::Configstring { index, value } => {
                    let output = {
                        let locked = inner.borrow();
                        Self::output_inner(&locked)
                    };
                    output.configstring(index, &value);
                }
            }
        }
        Ok(())
    }

    /// Notify a disconnect once (donor `notifyDisconnect`).
    fn notify_disconnect_inner(inner: &Rc<RefCell<Inner>>, slot: i32) {
        let notify = {
            let mut locked = inner.borrow_mut();
            let pending = match locked.clients.get_mut(&slot) {
                Some(entry) if !entry.disconnect_notified => {
                    entry.disconnect_notified = true;
                    Some(entry.player.actor.clone())
                }
                _ => None,
            };
            pending.and_then(|actor| locked.before_disconnect.clone().map(|callback| (callback, actor)))
        };
        if let Some((callback, actor)) = notify {
            callback(&actor);
        }
    }

    /// Mark an input slot retiring (donor `retireInput`).
    fn retire_input_inner(locked: &mut Inner, records: &Rc<Q3GuestRecords>, slot: i32, identity: &QvmClientIdentity) {
        if let Some(existing) = locked.retirements.get(&slot) {
            if existing.identity.actor != identity.actor || existing.identity.client != identity.client {
                panic!("QVM input retirement changed client generation");
            }
            return;
        }
        let ordinal = locked.clients.get(&slot).map(|entry| {
            if entry.player.actor != identity.actor || entry.player.client != identity.client {
                panic!("QVM input retirement does not own its source client");
            }
            entry.ordinal
        });
        records.retire_input_client(slot);
        locked.retirements.insert(
            slot,
            InputRetirement {
                identity: identity.clone(),
                ordinal,
                phase: if ordinal.is_some() {
                    RetirementPhase::Pending
                } else {
                    RetirementPhase::Complete
                },
            },
        );
    }

    /// Disconnect a retiring input slot (donor `disconnectInput`; the
    /// sync mirror always invokes `GAME_CLIENT_DISCONNECT` directly —
    /// there is no async `QvmFunctionCall` invocation to route).
    fn disconnect_input_inner(
        locked: &mut Inner,
        records: &Rc<Q3GuestRecords>,
        game: &QvmGame,
        slot: i32,
        identity: &QvmClientIdentity,
    ) -> Result<(), GuestError> {
        Self::retire_input_inner(locked, records, slot, identity);
        let entry_ordinal = {
            let retirement = locked
                .retirements
                .get_mut(&slot)
                .unwrap_or_else(|| panic!("QVM input retirement was lost"));
            if retirement.phase != RetirementPhase::Pending {
                return Ok(());
            }
            retirement.phase = RetirementPhase::Disconnecting;
            retirement.ordinal
        };
        if let Some(ordinal) = entry_ordinal {
            if let Some(entry) = locked.clients.get_mut(&slot) {
                if entry.ordinal == ordinal {
                    entry.phase = ClientPhase::Dropping {
                        reason: "Client input actor was removed.".to_string(),
                    };
                }
            }
        }
        let notify = entry_ordinal.and_then(|ordinal| {
            locked.clients.get(&slot).and_then(|entry| {
                if entry.ordinal == ordinal && !entry.disconnect_notified {
                    locked
                        .before_disconnect
                        .clone()
                        .map(|callback| (callback, entry.player.actor.clone()))
                } else {
                    None
                }
            })
        });
        if entry_ordinal.is_some() {
            if let Some(entry) = locked.clients.get_mut(&slot) {
                entry.disconnect_notified = true;
            }
        }
        if let Some((callback, actor)) = notify {
            callback(&actor);
        }
        game.client_disconnect(slot)?;
        locked
            .retirements
            .get_mut(&slot)
            .unwrap_or_else(|| panic!("QVM input retirement was lost"))
            .phase = RetirementPhase::Complete;
        Ok(())
    }

    /// Finish an external operation, draining complete retirements
    /// (donor `finishExternalOperation`).
    fn finish_external_operation(inner: &Rc<RefCell<Inner>>, records: &Rc<Q3GuestRecords>) {
        let complete: Vec<(i32, Option<u64>)> = {
            let mut locked = inner.borrow_mut();
            locked.external_ops -= 1;
            if locked.external_ops != 0 || locked.current_code.is_some() {
                return;
            }
            locked
                .retirements
                .iter()
                .filter(|(_, retirement)| retirement.phase == RetirementPhase::Complete)
                .map(|(slot, retirement)| (*slot, retirement.ordinal))
                .collect()
        };
        for (slot, ordinal) in complete {
            {
                let mut locked = inner.borrow_mut();
                if let Some(expected) = ordinal {
                    let current = locked.clients.get(&slot).map(|entry| entry.ordinal);
                    if current == Some(expected) {
                        Self::release_inner(&mut locked, slot);
                    }
                }
                locked.retirements.remove(&slot);
            }
            records.finish_input_retirement(slot);
        }
    }

    /// Run a body as an external operation.
    fn external<T>(&self, run: impl FnOnce() -> Result<T, GuestError>) -> Result<T, GuestError> {
        self.inner.borrow_mut().external_ops += 1;
        let result = run();
        Self::finish_external_operation(&self.inner, &self.records);
        result
    }

    /// Release a client entry (donor `release`).
    fn release_inner(locked: &mut Inner, slot: i32) {
        locked.clients.remove(&slot);
        locked.state.clear_client(slot);
        locked.records.release_client(slot);
    }

    /// Selected-game arsenal projection (donor `playerArsenal`).
    pub fn player_arsenal(&self, actor: &ActorId) -> Option<ArsenalState> {
        let locked = self.inner.borrow();
        Self::current_inner(&locked);
        locked.arsenal.clone().map(|arsenal| arsenal(actor))
    }

    /// Bind mod-client input (donor `bindInput`).
    pub fn bind_input(&self, definition: QvmInputDefinition, services: Rc<dyn QvmInputServices>) {
        let mut locked = self.inner.borrow_mut();
        if let Some(mut binding) = locked.input_binding.take() {
            binding.close();
        }
        let source = Rc::new(GuestInputSource {
            game: locked.game.clone(),
            definition,
            records: Rc::clone(&locked.records),
            inner: Rc::downgrade(&self.inner),
        });
        locked.input_binding = Some(QvmInputBinding::new(source, services));
    }

    /// Whether the guest retired (donor `isRetired`).
    #[must_use]
    pub fn is_retired(&self) -> bool {
        matches!(self.inner.borrow().lifecycle, Lifecycle::Retired)
    }

    /// Current time in milliseconds (donor `timeMilliseconds`).
    #[must_use]
    pub fn time_milliseconds(&self) -> i32 {
        let locked = self.inner.borrow();
        Self::current_inner(&locked);
        (locked.now)()
    }

    /// Publish a configstring when it changes (donor `setConfigstring`).
    pub fn set_configstring(&self, index: i32, value: &str) {
        let output = {
            let locked = self.inner.borrow();
            Self::current_inner(&locked);
            if locked.state.configstring_get(index) == value {
                return;
            }
            locked.state.configstring_set(index, value);
            Self::output_inner(&locked)
        };
        output.configstring(index, value);
    }

    /// Refresh the server-info configstring (donor `refreshServerInfo`).
    fn refresh_server_info(&self) {
        let (output, value) = {
            let locked = self.inner.borrow();
            Self::current_inner(&locked);
            let value = match locked.state.refresh_server_info() {
                Some(value) => value,
                None => return,
            };
            (Self::output_inner(&locked), value)
        };
        output.configstring(0, &value);
        Self::current_inner(&self.inner.borrow());
    }
}

impl Q3QvmServerGame {
    /// Initialize the game (donor `initialize`, sync port).
    pub fn initialize(&self, output: Rc<dyn Q3GuestOutput>, start: Q3GuestInitialization) -> Result<(), GuestError> {
        {
            let locked = self.inner.borrow();
            Self::current_inner(&locked);
            if !matches!(locked.lifecycle, Lifecycle::Created) {
                panic!("Q3 guest has already initialized");
            }
        }
        let carried: Option<(Vec<Q3GuestMapClient>, bool)> = match start {
            Q3GuestInitialization::New => None,
            Q3GuestInitialization::MapChange(clients) => Some((clients, false)),
            Q3GuestInitialization::MapRestart(clients) => Some((clients, true)),
        };
        if let Some((clients, _)) = &carried {
            let mut slots = HashSet::new();
            for entry in clients {
                self.validate_client(&entry.client);
                if !slots.insert(entry.client.slot()) {
                    panic!("Q3 map transition repeats a client slot");
                }
            }
            let mut locked = self.inner.borrow_mut();
            for entry in clients {
                locked.reconnecting.insert(entry.client.slot() as i32, entry.clone());
                locked.state.set_userinfo(entry.client.slot() as i32, &entry.userinfo);
            }
        }
        if self.state.cvars.borrow().variable_value("bot_enable") != 0.0 {
            panic!("Q3 guest bot services must be attached before initialization");
        }
        self.inner.borrow_mut().lifecycle = Lifecycle::Initializing {
            output: Rc::clone(&output),
        };
        let seed = self.inner.borrow().seed;
        let now = (self.inner.borrow().now)();
        let restart = matches!(carried, Some((_, true)));
        let result = self.game.initialize(now, seed, restart);
        match result {
            Ok(()) => {
                self.refresh_server_info();
                let mut locked = self.inner.borrow_mut();
                Self::current_inner(&locked);
                locked.lifecycle = Lifecycle::Running { output };
                Ok(())
            }
            Err(error) => match self.shutdown() {
                Ok(()) => Err(error),
                Err(cleanup) => Err(combine_errors(
                    "Q3 guest initialization and cleanup failed",
                    &[error.to_string(), cleanup.to_string()],
                )),
            },
        }
    }

    /// Validate a client slot (donor `validateClient`; session binding
    /// is structural — see the module docs).
    fn validate_client(&self, client: &ClientId) {
        let slot = client.slot() as i32;
        if slot < 0 || slot >= self.inner.borrow().max_clients {
            panic!("Q3 client slot belongs to another server or is outside its capacity");
        }
    }

    /// Connect a fresh client (donor `connect`).
    pub fn connect(&self, client: &ClientId, userinfo: &str) -> Result<Q3ApplicationAdmission, GuestError> {
        if self.inner.borrow().reconnecting.contains_key(&(client.slot() as i32)) {
            panic!("Q3 carried client requires map reconnection");
        }
        self.admit(client, userinfo, true, false)
    }

    /// Reconnect a map-carried client (donor `reconnect`).
    pub fn reconnect(&self, client: &ClientId) -> Result<Q3ApplicationAdmission, GuestError> {
        {
            let locked = self.inner.borrow();
            Self::running_inner(&locked);
        }
        let previous = self.inner.borrow().reconnecting.get(&(client.slot() as i32)).cloned();
        let Some(previous) = previous else {
            panic!("Q3 client is not carried by this map transition");
        };
        if previous.client != *client {
            panic!("Q3 client is not carried by this map transition");
        }
        let userinfo = self
            .state
            .get_userinfo(client.slot() as i32)
            .unwrap_or(previous.userinfo.clone());
        let bot = previous.bot;
        let result = self.admit(client, &userinfo, false, bot);
        self.inner.borrow_mut().reconnecting.remove(&(client.slot() as i32));
        result
    }

    /// Admit a client (donor `admit`).
    fn admit(
        &self,
        client: &ClientId,
        userinfo: &str,
        first_time: bool,
        is_bot: bool,
    ) -> Result<Q3ApplicationAdmission, GuestError> {
        self.external(|| {
            {
                let locked = self.inner.borrow();
                Self::running_inner(&locked);
            }
            if self.player(client).is_some() {
                panic!("Q3 client already has a guest slot");
            }
            let slot = client.slot() as i32;
            self.validate_client(client);
            let ordinal = {
                let mut locked = self.inner.borrow_mut();
                if locked.clients.contains_key(&slot) || locked.retirements.contains_key(&slot) {
                    return Ok(Q3ApplicationAdmission::Rejected {
                        reason: "Client slot is already occupied.".to_string(),
                    });
                }
                let ordinal = locked.next_ordinal;
                locked.next_ordinal += 1;
                let player = Q3ApplicationPlayer {
                    client: client.clone(),
                    actor: locked.records.actor(slot).id().clone(),
                    source_entity: slot,
                    admission: ordinal,
                };
                locked.clients.insert(
                    slot,
                    ClientEntry {
                        player,
                        ordinal,
                        disconnect_notified: false,
                        phase: ClientPhase::Connecting,
                    },
                );
                locked.state.set_userinfo(slot, userinfo);
                ordinal
            };
            let denied = match self.game.client_connect(slot, first_time, is_bot) {
                Ok(denied) => denied,
                Err(error) => {
                    let mut locked = self.inner.borrow_mut();
                    if locked.clients.get(&slot).is_some_and(|entry| entry.ordinal == ordinal) {
                        Self::release_inner(&mut locked, slot);
                    }
                    return Err(error);
                }
            };
            {
                let locked = self.inner.borrow();
                Self::current_inner(&locked);
            }
            let mut locked = self.inner.borrow_mut();
            let current = locked.clients.get(&slot);
            let admitted = current.is_some_and(|entry| entry.ordinal == ordinal);
            let dropping = locked.clients.get(&slot).and_then(|entry| match &entry.phase {
                ClientPhase::Dropping { reason } => Some(reason.clone()),
                _ => None,
            });
            if !admitted || dropping.is_some() {
                let reason = dropping.unwrap_or_else(|| "Client disconnected during admission.".to_string());
                return Ok(Q3ApplicationAdmission::Rejected { reason });
            }
            if let Some(denied) = denied {
                if first_time {
                    Self::release_inner(&mut locked, slot);
                } else {
                    drop(locked);
                    self.disconnect_inner_owned(slot, ordinal)?;
                    return Ok(Q3ApplicationAdmission::Rejected { reason: denied });
                }
                return Ok(Q3ApplicationAdmission::Rejected { reason: denied });
            }
            if let Some(entry) = locked.clients.get_mut(&slot) {
                entry.phase = ClientPhase::Connected;
            }
            let player = locked.clients.get(&slot).map(|entry| entry.player.clone());
            drop(locked);
            match player {
                Some(player) => Ok(Q3ApplicationAdmission::Accepted { player }),
                None => Ok(Q3ApplicationAdmission::Rejected {
                    reason: "Client disconnected during admission.".to_string(),
                }),
            }
        })
    }

    /// Look up an admitted entry (donor `entry`).
    fn entry_inner(locked: &Inner, player: &Q3ApplicationPlayer) -> u64 {
        match locked.clients.get(&player.source_entity) {
            Some(entry) if entry.player == *player => entry.ordinal,
            _ => panic!("Q3 player is no longer admitted"),
        }
    }

    /// Begin an admitted client (donor `begin`).
    pub fn begin(&self, player: &Q3ApplicationPlayer, command: &Q3StoredUserCommand) -> Result<(), GuestError> {
        self.external(|| {
            let ordinal = {
                let locked = self.inner.borrow();
                Self::running_inner(&locked);
                Self::entry_inner(&locked, player)
            };
            {
                let locked = self.inner.borrow();
                let phase = locked
                    .clients
                    .get(&player.source_entity)
                    .map(|entry| entry.phase.clone());
                if phase != Some(ClientPhase::Connected) {
                    panic!("Q3 guest client cannot begin in this phase");
                }
                locked.state.set_user_command(player.source_entity, *command);
            }
            self.game.client_begin(player.source_entity)?;
            let notify = {
                let mut locked = self.inner.borrow_mut();
                Self::current_inner(&locked);
                let admitted = match locked.clients.get_mut(&player.source_entity) {
                    Some(entry) if entry.ordinal == ordinal && entry.phase == ClientPhase::Connected => {
                        entry.phase = ClientPhase::Active;
                        Some(entry.player.actor.clone())
                    }
                    _ => None,
                };
                admitted.and_then(|actor| locked.client_changed.clone().map(|callback| (callback, actor)))
            };
            if let Some((callback, actor)) = notify {
                callback(Q3ClientChangedKind::Admitted, &actor);
            }
            Ok(())
        })
    }

    /// Run a client think (donor `think`).
    pub fn think(&self, player: &Q3ApplicationPlayer, command: &Q3StoredUserCommand) -> Result<(), GuestError> {
        self.external(|| {
            {
                let locked = self.inner.borrow();
                Self::running_inner(&locked);
                let ordinal = Self::entry_inner(&locked, player);
                let active = locked
                    .clients
                    .get(&player.source_entity)
                    .is_some_and(|entry| entry.ordinal == ordinal && entry.phase == ClientPhase::Active);
                if !active {
                    panic!("Q3 guest client is not active");
                }
                locked.state.set_user_command(player.source_entity, *command);
            }
            self.game.client_think(player.source_entity)?;
            let locked = self.inner.borrow();
            Self::current_inner(&locked);
            Ok(())
        })
    }

    /// Report a userinfo change (donor `userinfo`).
    pub fn userinfo(&self, player: &Q3ApplicationPlayer, value: &str) -> Result<(), GuestError> {
        self.external(|| {
            {
                let locked = self.inner.borrow();
                Self::running_inner(&locked);
                Self::entry_inner(&locked, player);
                locked.state.set_userinfo(player.source_entity, value);
            }
            self.game.client_userinfo_changed(player.source_entity)?;
            let notify = {
                let locked = self.inner.borrow();
                Self::current_inner(&locked);
                locked
                    .client_changed
                    .clone()
                    .map(|callback| (callback, player.actor.clone()))
            };
            if let Some((callback, actor)) = notify {
                callback(Q3ClientChangedKind::Userinfo, &actor);
            }
            Ok(())
        })
    }

    /// Deliver a client command immediately (donor `commandImmediate`).
    pub fn command_immediate(&self, player: &Q3ApplicationPlayer, argv: &[String]) -> Result<(), GuestError> {
        self.external(|| {
            {
                let locked = self.inner.borrow();
                Self::running_inner(&locked);
                Self::entry_inner(&locked, player);
            }
            self.game.client_command(player.source_entity, argv)?;
            let locked = self.inner.borrow();
            Self::current_inner(&locked);
            Ok(())
        })
    }

    /// Deliver a client command (donor `command`; identical to
    /// `command_immediate` in the sync port).
    pub fn command(&self, player: &Q3ApplicationPlayer, argv: &[String]) -> Result<(), GuestError> {
        self.command_immediate(player, argv)
    }

    /// Drop a client from a trap (donor `drop`).
    fn drop_inner(inner: &Rc<RefCell<Inner>>, slot: i32, reason: &str) -> Result<(), GuestError> {
        let watched = {
            let mut locked = inner.borrow_mut();
            locked.reconnecting.remove(&slot);
            match locked.clients.get_mut(&slot) {
                Some(entry) => {
                    entry.phase = ClientPhase::Dropping {
                        reason: reason.to_string(),
                    };
                    Some((entry.player.clone(), entry.ordinal))
                }
                None => None,
            }
        };
        if watched.is_some() {
            Self::notify_disconnect_inner(inner, slot);
        }
        let output = {
            let locked = inner.borrow();
            Self::output_inner(&locked)
        };
        output.drop_client(slot, reason);
        {
            let locked = inner.borrow();
            Self::current_inner(&locked);
        }
        if let Some((player, ordinal)) = watched {
            let current = inner.borrow().clients.get(&slot).map(|entry| entry.ordinal);
            if current == Some(ordinal) {
                Self::disconnect_player_inner(inner, &player)?;
            }
        }
        Ok(())
    }

    /// Disconnect a player by slot/ordinal identity.
    fn disconnect_inner_owned(&self, slot: i32, ordinal: u64) -> Result<(), GuestError> {
        let player = self
            .inner
            .borrow()
            .clients
            .get(&slot)
            .filter(|entry| entry.ordinal == ordinal)
            .map(|entry| entry.player.clone());
        match player {
            Some(player) => self.disconnect(&player),
            None => Ok(()),
        }
    }

    /// Disconnect a player (donor `disconnect`).
    pub fn disconnect(&self, player: &Q3ApplicationPlayer) -> Result<(), GuestError> {
        Self::disconnect_player_inner(&self.inner, player)
    }

    /// Shared disconnect implementation.
    fn disconnect_player_inner(inner: &Rc<RefCell<Inner>>, player: &Q3ApplicationPlayer) -> Result<(), GuestError> {
        inner.borrow_mut().external_ops += 1;
        let result = (|| -> Result<(), GuestError> {
            {
                let locked = inner.borrow();
                Self::current_inner(&locked);
                match &locked.lifecycle {
                    Lifecycle::Initializing { .. } | Lifecycle::Running { .. } | Lifecycle::ShuttingDown { .. } => {}
                    _ => panic!("Q3 guest client disconnect requires attached output"),
                }
                let admitted = locked
                    .clients
                    .get(&player.source_entity)
                    .is_some_and(|entry| entry.player == *player);
                if !admitted {
                    return Ok(());
                }
            }
            Self::notify_disconnect_inner(inner, player.source_entity);
            let game = {
                let mut locked = inner.borrow_mut();
                if let Some(entry) = locked.clients.get_mut(&player.source_entity) {
                    if !matches!(entry.phase, ClientPhase::Dropping { .. }) {
                        entry.phase = ClientPhase::Dropping {
                            reason: "Client disconnected.".to_string(),
                        };
                    }
                }
                locked.clients.remove(&player.source_entity);
                locked.game.clone()
            };
            // The sync mirror invokes `GAME_CLIENT_DISCONNECT` directly:
            // `module.call` never reenters the trap host.
            let result = game.client_disconnect(player.source_entity);
            {
                let locked = inner.borrow();
                Self::current_inner(&locked);
                locked.state.clear_client(player.source_entity);
                locked.records.release_client(player.source_entity);
            }
            result
        })();
        let records = inner.borrow().records.clone();
        Self::finish_external_operation(inner, &records);
        result
    }

    /// Run a console command (donor `consoleCommand`).
    pub fn console_command(&self, argv: &[String]) -> Result<bool, GuestError> {
        self.external(|| {
            {
                let locked = self.inner.borrow();
                Self::running_inner(&locked);
            }
            let handled = self.game.console_command(argv)?;
            let locked = self.inner.borrow();
            Self::current_inner(&locked);
            Ok(handled)
        })
    }

    /// Run a server frame (donor `runFrame`).
    pub fn run_frame(&self, time_milliseconds: i32) -> Result<(), GuestError> {
        self.external(|| {
            {
                let locked = self.inner.borrow();
                Self::running_inner(&locked);
            }
            self.game.run_frame(time_milliseconds)?;
            self.refresh_server_info();
            let locked = self.inner.borrow();
            Self::current_inner(&locked);
            Ok(())
        })
    }

    /// Whether a client is a bot (always false without botlib).
    #[must_use]
    pub fn is_bot(&self, client: &ClientId) -> bool {
        let _ = client;
        false
    }

    /// Live players (donor `players`).
    #[must_use]
    pub fn players(&self) -> Vec<Q3ApplicationPlayer> {
        let locked = self.inner.borrow();
        locked
            .clients
            .values()
            .filter(|entry| {
                !matches!(entry.phase, ClientPhase::Dropping { .. })
                    && !locked.retirements.contains_key(&entry.player.source_entity)
            })
            .map(|entry| entry.player.clone())
            .collect()
    }

    /// Look up a player by client (donor `player`).
    #[must_use]
    pub fn player(&self, client: &ClientId) -> Option<Q3ApplicationPlayer> {
        self.players().into_iter().find(|player| player.client == *client)
    }
}

/// Capture the token stream as `(cursor, parser)` save images.
fn capture_tokens(tokens: &QvmEntityTokens) -> (SaveJson, SaveJson) {
    let state = tokens.capture_save_state();
    let cursor = match state.record_get("cursor") {
        Some(ProfileValue::Int(offset)) => int(*offset),
        _ => SaveJson::Null,
    };
    let parser = state.record_get("parser");
    let field = |name: &str| parser.and_then(|parser| parser.record_get(name));
    let text = |name: &str| match field(name) {
        Some(ProfileValue::Str(value)) => value.clone(),
        _ => String::new(),
    };
    let line = match field("line") {
        Some(ProfileValue::Int(line)) => *line,
        _ => 0,
    };
    (
        cursor,
        obj(vec![
            ("token", save_str(&text("token"))),
            ("line", int(line)),
            ("name", save_str(&text("name"))),
        ]),
    )
}

/// Require the value behind a reader.
fn required<'a>(reader: &SaveReader<'a>) -> Result<&'a SaveJson, WorldError> {
    reader.value.ok_or_else(|| reader.fail("expected a value"))
}

/// Restore the token stream from `(cursor, parser)` save images.
fn restore_tokens(
    tokens: &mut QvmEntityTokens,
    entity_text: &str,
    cursor: &SaveReader,
    parser: &SaveReader,
) -> Result<(), GuestError> {
    let offset = cursor
        .nullable(|cell| cell.integer(0))
        .map_err(|error| GuestError::invalid(error.to_string()))?;
    let state = ProfileValue::record(vec![
        ("source", ProfileValue::Bytes(entity_text.as_bytes().to_vec())),
        ("cursor", offset.map_or(ProfileValue::Null, ProfileValue::Int)),
        (
            "parser",
            ProfileValue::record(vec![
                (
                    "token",
                    ProfileValue::Str(
                        parser
                            .field("token")
                            .string()
                            .map_err(|error| GuestError::invalid(error.to_string()))?,
                    ),
                ),
                (
                    "line",
                    ProfileValue::Int(
                        parser
                            .field("line")
                            .integer(0)
                            .map_err(|error| GuestError::invalid(error.to_string()))?,
                    ),
                ),
                (
                    "name",
                    ProfileValue::Str(
                        parser
                            .field("name")
                            .string()
                            .map_err(|error| GuestError::invalid(error.to_string()))?,
                    ),
                ),
            ]),
        ),
    ]);
    tokens.restore_save_state(&state)
}

impl Q3QvmServerGame {
    /// Capture the host save image (donor `captureHost`).
    fn capture_host(locked: &Inner) -> Result<SaveJson, GuestError> {
        Self::running_inner(locked);
        let settled = locked.external_ops == 0
            && locked.current_code.is_none()
            && locked.reconnecting.is_empty()
            && locked.retirements.is_empty()
            && locked
                .clients
                .values()
                .all(|entry| matches!(entry.phase, ClientPhase::Connected | ClientPhase::Active));
        if !settled {
            panic!("Q3 guest checkpoint requires completed client operations");
        }
        let data = locked.game.data.checkpoint();
        let (cursor_image, parser_image) = capture_tokens(&locked.tokens);
        let files = locked
            .files
            .capture_checkpoint()
            .map_err(|error| GuestError::invalid(error.to_string()))?;
        let clients = locked
            .clients
            .values()
            .map(|entry| {
                let phase = match entry.phase {
                    ClientPhase::Connected => "connected",
                    ClientPhase::Active => "active",
                    _ => panic!("Q3 guest checkpoint requires completed client operations"),
                };
                obj(vec![
                    ("sourceEntity", int(i64::from(entry.player.source_entity))),
                    (
                        "client",
                        obj(vec![
                            ("slot", int(i64::from(entry.player.client.slot()))),
                            ("generation", int(i64::from(entry.player.client.generation()))),
                        ]),
                    ),
                    ("actor", write_saved_actor(SavedActorId::from(&entry.player.actor))),
                    ("phase", save_str(phase)),
                ])
            })
            .collect();
        Ok(obj(vec![
            ("version", int(HOST_VERSION)),
            ("maxClients", int(locked.game.data.num_clients() as i64)),
            (
                "data",
                obj(vec![
                    ("entitiesWord", int(data.entities_word as i64)),
                    ("numEntities", int(data.num_entities as i64)),
                    ("entityStride", int(data.entity_stride as i64)),
                    ("clientsWord", int(data.clients_word as i64)),
                    ("clientStride", int(data.client_stride as i64)),
                ]),
            ),
            ("server", locked.state.capture_save_state()),
            ("entityText", save_str(&locked.entity_text)),
            ("cursor", cursor_image),
            ("parser", parser_image),
            ("bots", SaveJson::Null),
            ("botClients", arr(Vec::new())),
            ("botMessages", arr(Vec::new())),
            ("records", locked.records.capture_checkpoint()),
            ("portals", locked.spatial.world.capture_portal_checkpoint()),
            ("files", encode_files(&files)),
            ("clients", arr(clients)),
        ]))
    }

    /// Capture a module checkpoint (donor `checkpoint`).
    pub fn checkpoint(&self) -> Result<QvmCheckpoint, GuestError> {
        let host = {
            let locked = self.inner.borrow();
            Self::capture_host(&locked)?
        };
        let mut checkpoint = self.game.module.checkpoint()?;
        checkpoint.host_state = QvmHostState {
            module: checkpoint.module.clone(),
            format: HOST_FORMAT.to_string(),
            bytes: ProfileValue::Bytes(encode_checkpoint_value(&host)),
        };
        Ok(checkpoint)
    }

    /// Restore a module checkpoint (donor `restoreCheckpoint`).
    pub fn restore_checkpoint(
        &self,
        checkpoint: &QvmCheckpoint,
        resolve_client: impl Fn(&Q3SavedGuestClientId) -> ClientId + 'static,
    ) -> Result<(), GuestError> {
        {
            let locked = self.inner.borrow();
            Self::current_inner(&locked);
            if !matches!(locked.lifecycle, Lifecycle::Created) {
                panic!("Q3 guest restore requires an inert candidate");
            }
        }
        self.inner.borrow_mut().lifecycle = Lifecycle::Restoring;
        self.inner.borrow_mut().restore_client = Some(Rc::new(resolve_client));
        let result = self
            .game
            .module
            .restore(checkpoint)
            .and_then(|()| Self::restore_host(&self.inner, checkpoint));
        self.inner.borrow_mut().restore_client = None;
        match result {
            Ok(()) => {
                self.inner.borrow_mut().lifecycle = Lifecycle::Restored;
                Ok(())
            }
            Err(error) => match self.discard() {
                Ok(()) => Err(error),
                Err(cleanup) => Err(combine_errors(
                    "Q3 guest restore and discard failed",
                    &[error.to_string(), cleanup.to_string()],
                )),
            },
        }
    }

    /// Restore the host save image (donor `restoreHost`).
    fn restore_host(inner: &Rc<RefCell<Inner>>, checkpoint: &QvmCheckpoint) -> Result<(), GuestError> {
        let fail = |message: &str| GuestError::invalid(format!("q3.guest.host: {message}"));
        let resolve_client = {
            let locked = inner.borrow();
            if !matches!(locked.lifecycle, Lifecycle::Restoring) || locked.restore_client.is_none() {
                panic!("Q3 guest host restore requires the candidate restore operation");
            }
            locked
                .restore_client
                .clone()
                .expect("Q3 guest host restore requires the candidate restore operation")
        };
        if checkpoint.host_state.format != HOST_FORMAT {
            return Err(fail("unsupported Q3 guest host checkpoint format"));
        }
        let ProfileValue::Bytes(bytes) = &checkpoint.host_state.bytes else {
            return Err(fail("unsupported Q3 guest host checkpoint format"));
        };
        let value = decode_checkpoint_value(bytes).map_err(|error| fail(&error.to_string()))?;
        let reader = SaveReader::at(&value, "q3.guest.host");
        reader
            .field("version")
            .literal_i64(HOST_VERSION)
            .map_err(|_| fail("version mismatch"))?;
        let max_clients = inner.borrow().max_clients;
        reader
            .field("maxClients")
            .literal_i64(i64::from(max_clients))
            .map_err(|_| fail("guest client capacity differs"))?;
        let entity_text = inner.borrow().entity_text.clone();
        reader
            .field("entityText")
            .literal_str(&entity_text)
            .map_err(|_| fail("entity text differs"))?;
        {
            let locked = inner.borrow();
            if let Some(source_restored) = locked.source_restored.clone() {
                drop(locked);
                source_restored();
            }
        }
        let data = reader.field("data");
        let tables = QvmGameDataState {
            entities_word: data
                .field("entitiesWord")
                .integer(0)
                .map_err(|error| fail(&error.to_string()))? as usize,
            num_entities: data
                .field("numEntities")
                .integer(0)
                .map_err(|error| fail(&error.to_string()))? as usize,
            entity_stride: data
                .field("entityStride")
                .integer(0)
                .map_err(|error| fail(&error.to_string()))? as usize,
            clients_word: data
                .field("clientsWord")
                .integer(0)
                .map_err(|error| fail(&error.to_string()))? as usize,
            client_stride: data
                .field("clientStride")
                .integer(0)
                .map_err(|error| fail(&error.to_string()))? as usize,
        };
        {
            let locked = inner.borrow();
            locked.game.data.set_client_count(max_clients as usize)?;
            locked.game.data.restore(&tables)?;
            locked
                .state
                .restore_save_state(required(&reader.field("server")).map_err(|error| fail(&error.to_string()))?)
                .map_err(|error| fail(&error.to_string()))?;
        }
        {
            let locked = inner.borrow();
            Self::current_inner(&locked);
        }
        {
            let mut locked = inner.borrow_mut();
            restore_tokens(
                &mut locked.tokens,
                &entity_text,
                &reader.field("cursor"),
                &reader.field("parser"),
            )?;
            locked
                .spatial
                .world
                .restore_portal_checkpoint(
                    required(&reader.field("portals")).map_err(|error| fail(&error.to_string()))?,
                )
                .map_err(|error| fail(&error.to_string()))?;
            locked
                .records
                .restore_checkpoint(required(&reader.field("records")).map_err(|error| fail(&error.to_string()))?)
                .map_err(|error| fail(&error.to_string()))?;
        }
        let clients = read_guest_clients(&reader.field("clients")).map_err(|error| fail(&error.to_string()))?;
        {
            let mut locked = inner.borrow_mut();
            for saved in &clients {
                let actor = locked.records.host.actors.resolve_saved(&saved.actor);
                let client = resolve_client(&saved.client);
                let bound = saved.source_entity < max_clients
                    && saved.source_entity == saved.client.slot as i32
                    && !locked.clients.contains_key(&saved.source_entity)
                    && client.slot() == saved.client.slot
                    && actor
                        .as_ref()
                        .is_some_and(|actor| locked.records.slot(actor.id()) == Some(saved.source_entity));
                let Some(actor) = actor else {
                    return Err(fail("invalid restored guest client binding"));
                };
                if !bound {
                    return Err(fail("invalid restored guest client binding"));
                }
                let ordinal = locked.next_ordinal;
                locked.next_ordinal += 1;
                locked.clients.insert(
                    saved.source_entity,
                    ClientEntry {
                        player: Q3ApplicationPlayer {
                            client,
                            actor: actor.id().clone(),
                            source_entity: saved.source_entity,
                            admission: ordinal,
                        },
                        ordinal,
                        disconnect_notified: false,
                        phase: match saved.phase {
                            Q3SavedClientPhase::Connected => ClientPhase::Connected,
                            Q3SavedClientPhase::Active => ClientPhase::Active,
                        },
                    },
                );
            }
        }
        {
            let mut locked = inner.borrow_mut();
            let files = decode_files(&reader.field("files")).map_err(|error| fail(&error.to_string()))?;
            locked.files.restore_checkpoint(&files)?;
            if !reader.field("bots").is_missing() && !matches!(reader.field("bots").value, Some(SaveJson::Null)) {
                return Err(fail("saved guest bot services need the bot lane"));
            }
            if !reader.field("botClients").is_missing() {
                let slots = reader
                    .field("botClients")
                    .list(|cell| cell.integer(0))
                    .map_err(|error| fail(&error.to_string()))?;
                if !slots.is_empty() {
                    return Err(fail("saved guest bot services need the bot lane"));
                }
            }
            if !reader.field("botMessages").is_missing() {
                let messages = reader
                    .field("botMessages")
                    .list(|cell| cell.field("slot").integer(0))
                    .map_err(|error| fail(&error.to_string()))?;
                if !messages.is_empty() {
                    return Err(fail("saved guest bot services need the bot lane"));
                }
            }
        }
        Ok(())
    }

    /// Attach output after a restore (donor `completeRestore`).
    pub fn complete_restore(&self, output: Rc<dyn Q3GuestOutput>) {
        let mut locked = self.inner.borrow_mut();
        Self::current_inner(&locked);
        if !matches!(locked.lifecycle, Lifecycle::Restored) {
            panic!("Q3 guest restore is not ready for output attachment");
        }
        locked.lifecycle = Lifecycle::Running { output };
    }

    /// Discard the guest without shutting the game down (donor
    /// `discard`).
    pub fn discard(&self) -> Result<(), GuestError> {
        {
            let locked = self.inner.borrow();
            if matches!(locked.lifecycle, Lifecycle::Retired) {
                return Ok(());
            }
            if locked.external_ops != 0 || locked.current_code.is_some() {
                panic!("Q3 guest discard must await the active operation");
            }
        }
        {
            let mut locked = self.inner.borrow_mut();
            if let Some(mut binding) = locked.input_binding.take() {
                binding.close();
            }
            locked.retirements.clear();
        }
        let mut errors: Vec<String> = Vec::new();
        {
            let mut locked = self.inner.borrow_mut();
            locked.files.close_all();
            if let Some(before_retire) = locked.before_retire.clone() {
                drop(locked);
                Self::catch_cleanup(&mut errors, before_retire);
            }
        }
        {
            let locked = self.inner.borrow();
            locked.records.close();
        }
        self.game.retire();
        {
            let mut locked = self.inner.borrow_mut();
            locked.clients.clear();
            locked.reconnecting.clear();
            locked.lifecycle = Lifecycle::Retired;
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(combine_errors("Q3 guest discard failed", &errors))
        }
    }

    /// Shut the game down (donor `shutdown`).
    pub fn shutdown(&self) -> Result<(), GuestError> {
        {
            let locked = self.inner.borrow();
            if matches!(locked.lifecycle, Lifecycle::Retired) {
                return Ok(());
            }
            if locked.external_ops != 0 || locked.current_code.is_some() {
                panic!("Q3 guest shutdown must await the active operation");
            }
        }
        let mut errors: Vec<String> = Vec::new();
        if let Err(error) = self.shutdown_source(false) {
            errors.push(error.to_string());
        }
        Self::release_source(&self.inner, &mut errors, false);
        if errors.is_empty() {
            Ok(())
        } else {
            Err(combine_errors("Q3 guest shutdown failed", &errors))
        }
    }

    /// Shut down for a map change, capturing carried state (donor
    /// `shutdownForMapChange`).
    pub fn shutdown_for_map_change(&self, restart: bool) -> Result<Q3GuestMapTransition, GuestError> {
        {
            let locked = self.inner.borrow();
            Self::running_inner(&locked);
            let settled = locked.external_ops == 0
                && locked.current_code.is_none()
                && locked.reconnecting.is_empty()
                && locked
                    .clients
                    .values()
                    .all(|entry| matches!(entry.phase, ClientPhase::Connected | ClientPhase::Active));
            if !settled {
                panic!("Q3 map transition requires completed client operations");
            }
        }
        let mut errors: Vec<String> = Vec::new();
        let transition = match self.shutdown_source(restart) {
            Ok(()) => {
                let locked = self.inner.borrow();
                Some(Q3GuestMapTransition {
                    cvars: locked.state.capture_save_state(),
                    clients: locked
                        .clients
                        .values()
                        .filter(|entry| !matches!(entry.phase, ClientPhase::Dropping { .. }))
                        .map(|entry| Q3GuestMapClient {
                            client: entry.player.client.clone(),
                            userinfo: locked
                                .state
                                .get_userinfo(entry.player.source_entity)
                                .unwrap_or_default(),
                            bot: false,
                        })
                        .collect(),
                })
            }
            Err(error) => {
                errors.push(error.to_string());
                None
            }
        };
        Self::release_source(&self.inner, &mut errors, transition.is_some());
        if !errors.is_empty() {
            return Err(combine_errors("Q3 guest map shutdown failed", &errors));
        }
        Ok(transition.expect("Q3 map transition did not capture source state"))
    }

    /// Shut the game source down (donor `shutdownSource`).
    fn shutdown_source(&self, restart: bool) -> Result<(), GuestError> {
        {
            let mut locked = self.inner.borrow_mut();
            if let Some(mut binding) = locked.input_binding.take() {
                binding.close();
            }
            locked.retirements.clear();
            let output = match &locked.lifecycle {
                Lifecycle::Initializing { output }
                | Lifecycle::Running { output }
                | Lifecycle::ShuttingDown { output } => Some(Rc::clone(output)),
                _ => None,
            };
            if let Some(output) = output {
                locked.lifecycle = Lifecycle::ShuttingDown { output };
            } else {
                return Ok(());
            }
        }
        self.game.shutdown(restart)
    }

    /// Run a retire callback, collecting panics as cleanup errors.
    fn catch_cleanup(errors: &mut Vec<String>, callback: Q3GuestThunk) {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| callback()));
        if let Err(payload) = result {
            let message = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|text| text.to_string()))
                .unwrap_or_else(|| "retire callback failed".to_string());
            errors.push(message);
        }
    }

    /// Release source resources (donor `releaseSource`).
    fn release_source(inner: &Rc<RefCell<Inner>>, errors: &mut Vec<String>, _transfer_clients: bool) {
        {
            let mut locked = inner.borrow_mut();
            locked.files.close_all();
            if let Some(before_retire) = locked.before_retire.clone() {
                drop(locked);
                Self::catch_cleanup(errors, before_retire);
            }
        }
        {
            let locked = inner.borrow();
            locked.records.close();
        }
        {
            let mut locked = inner.borrow_mut();
            let slots: Vec<i32> = locked
                .clients
                .keys()
                .chain(locked.reconnecting.keys())
                .copied()
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();
            for slot in slots {
                locked.state.clear_client(slot);
            }
            locked.game.retire();
            locked.clients.clear();
            locked.reconnecting.clear();
            locked.lifecycle = Lifecycle::Retired;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::guest_records::{
        Q3GuestActorSource, Q3GuestBodyBinding, Q3GuestLeafQuery, Q3GuestModelTraceQuery, Q3GuestPointTarget,
        Q3GuestRecordActors, Q3GuestRecordBodies, Q3GuestScene, Q3GuestTraceQuery,
    };
    use super::super::guest_world::{Q3GuestGeometry, Q3GuestGeometryKind, Q3GuestNativeClipWorld, Q3GuestTopology};
    use super::super::host::Q3HostSettings;
    use super::*;
    use qa_content::contract::{create_mount_plan_id, ResolvedMountPlan};
    use qa_content::mounts::{open_mount_plan, OpenMountOptions};
    use qa_content::q3::base::world::ActorTraceResult;
    use qa_core::identity::{IdentityOwner, OwnedActor, ProviderId};
    use qa_core::math::{Bounds, Vec3};
    use qa_guest::qvm::game_data::{ModuleIdentity, QvmGameExport, QvmImage};
    use qa_guest::qvm::game_input::{
        QvmApplicationInput, QvmClientApplication, QvmClientApplications, QvmClientCommand, QvmFrameContext,
        QvmFramePhase, QvmInputEntries, QvmReleaseHandle, QvmReleaseListener, QvmSourceTime, QvmTimeKind,
    };

    struct FakeActors {
        owner: IdentityOwner,
        owned: RefCell<HashMap<ActorId, OwnedActor>>,
        sources: RefCell<HashMap<ActorId, Q3GuestActorSource>>,
    }

    impl FakeActors {
        fn new(name: &str) -> Self {
            Self {
                owner: IdentityOwner::create(name).unwrap(),
                owned: RefCell::new(HashMap::new()),
                sources: RefCell::new(HashMap::new()),
            }
        }
    }

    impl Q3GuestRecordActors for FakeActors {
        fn assert_owned(&self, actor: &OwnedActor) {
            assert!(self.owner.owns_owned(actor));
        }

        fn allocate_at_source(&self, provider: &ProviderId, slot: usize, _definition: &str) -> OwnedActor {
            let id = self.owner.actor(slot as u32, 1);
            let owned = self.owner.owned_actor(&id, provider.clone()).unwrap();
            self.owned.borrow_mut().insert(id.clone(), owned.clone());
            self.sources.borrow_mut().insert(
                id,
                Q3GuestActorSource {
                    provider: provider.clone(),
                    slot,
                },
            );
            owned
        }

        fn source_of(&self, actor: &ActorId) -> Option<Q3GuestActorSource> {
            self.sources.borrow().get(actor).cloned()
        }

        fn at_source(&self, _provider: &ProviderId, slot: usize) -> Option<OwnedActor> {
            let sources = self.sources.borrow();
            let id = sources
                .iter()
                .find(|(_, source)| source.slot == slot)
                .map(|(id, _)| id.clone())?;
            drop(sources);
            self.owned.borrow().get(&id).cloned()
        }

        fn owned_by(&self, provider: &ProviderId) -> Vec<OwnedActor> {
            self.owned
                .borrow()
                .values()
                .filter(|actor| actor.owner() == provider)
                .cloned()
                .collect()
        }

        fn resolve_saved(&self, saved: &SavedActorId) -> Option<OwnedActor> {
            self.owned
                .borrow()
                .values()
                .find(|actor| SavedActorId::from(actor.id()) == *saved)
                .cloned()
        }

        fn on_release(&self, _callback: Box<dyn Fn(&OwnedActor)>) -> Box<dyn Fn()> {
            Box::new(|| {})
        }

        fn release(&self, actor: &OwnedActor) {
            self.owned.borrow_mut().remove(actor.id());
            self.sources.borrow_mut().remove(actor.id());
        }

        fn session(&self) -> qa_core::identity::SessionId {
            self.owner.session().clone()
        }
    }

    struct FakeBodies;

    impl Q3GuestRecordBodies for FakeBodies {
        fn bind(&self, _actor: &OwnedActor, _binding: Q3GuestBodyBinding) {}
        fn unlink(&self, _actor: &OwnedActor) {}
        fn link(&self, _actor: &OwnedActor) {}
    }

    struct FakeScene;

    impl Q3GuestTopology for FakeScene {
        fn geometry(&self) -> Q3GuestGeometry {
            Q3GuestGeometry {
                kind: Q3GuestGeometryKind::Q3Bsp,
                areas: Vec::new(),
                area_portals: Vec::new(),
                leaf_count: 1,
            }
        }

        fn adjust_area_portal_state(&self, _first: i32, _second: i32, _open: bool) {}

        fn adjust_area_portal_contribution(&self, _portal: i32, _delta: i32) {}

        fn native_q3_clip_models(&self) -> Option<Rc<dyn Q3GuestNativeClipWorld>> {
            None
        }
    }

    impl Q3GuestScene for FakeScene {
        fn bind_actor_collision(&self, _read: super::super::guest_records::Q3GuestCollisionReader) {}
        fn box_leaves(&self, _bounds: &Bounds, _limit: usize) -> Q3GuestLeafQuery {
            Q3GuestLeafQuery {
                leaves: vec![0],
                topnode: None,
                overflow: false,
            }
        }
        fn leaf_cluster(&self, _leaf: i32) -> i32 {
            0
        }
        fn leaf_area(&self, _leaf: i32) -> i32 {
            0
        }
        fn trace(&self, _query: &Q3GuestTraceQuery) -> ActorTraceResult {
            panic!("unused in guest-runtime tests");
        }
        fn point_contents(&self, _point: Vec3, _target: &Q3GuestPointTarget, _pass_actor: Option<&ActorId>) -> i32 {
            panic!("unused in guest-runtime tests");
        }
        fn query_actors(&self, _bounds: &Bounds) -> Vec<ActorId> {
            Vec::new()
        }
        fn geometry_trace(&self, _query: &Q3GuestModelTraceQuery) -> ActorTraceResult {
            panic!("unused in guest-runtime tests");
        }
        fn model_bounds(&self, _model: i32) -> Bounds {
            panic!("unused in guest-runtime tests");
        }
        fn areas_connected(&self, _first: i32, _second: i32) -> bool {
            true
        }
        fn point_leaf(&self, _point: Vec3) -> i32 {
            0
        }
        fn cluster_visible(&self, _from: i32, _cluster: i32) -> bool {
            true
        }
    }

    struct FakeClip;

    impl super::super::guest_spatial::Q3GuestClipModel for FakeClip {
        fn transformed_point_contents(&self, _point: Vec3, _origin: Vec3, _angles: Vec3) -> i32 {
            0
        }
        fn transformed_trace_solid(
            &self,
            _start: Vec3,
            _end: Vec3,
            _mask: i32,
            _shape: qa_world::movement::types::TraceShape,
            _origin: Vec3,
            _angles: Vec3,
        ) -> bool {
            false
        }
    }

    struct FakeOutput {
        drops: RefCell<Vec<(i32, String)>>,
        commands: RefCell<Vec<(i32, String)>>,
        configstrings: RefCell<Vec<(i32, String)>>,
    }

    impl FakeOutput {
        fn new() -> Self {
            Self {
                drops: RefCell::new(Vec::new()),
                commands: RefCell::new(Vec::new()),
                configstrings: RefCell::new(Vec::new()),
            }
        }
    }

    impl Q3GuestOutput for FakeOutput {
        fn drop_client(&self, slot: i32, reason: &str) {
            self.drops.borrow_mut().push((slot, reason.to_string()));
        }
        fn send_server_command(&self, slot: i32, text: &str) {
            self.commands.borrow_mut().push((slot, text.to_string()));
        }
        fn configstring(&self, index: i32, value: &str) {
            self.configstrings.borrow_mut().push((index, value.to_string()));
        }
    }

    struct FakeApps;

    impl QvmClientApplications for FakeApps {
        fn active(&self) -> bool {
            false
        }
        fn begin(
            &self,
            _input: &QvmApplicationInput,
            _encode_aim: &mut dyn FnMut(
                &Vec3,
                &qa_guest::qvm::game_input::Q3UserCommand,
            ) -> Result<qa_guest::qvm::game_input::Q3UserCommand, GuestError>,
        ) -> Result<Option<QvmClientApplication>, GuestError> {
            Ok(None)
        }
        fn finish(&self, _application: Option<&QvmClientApplication>, _failed: bool) -> Result<(), GuestError> {
            Ok(())
        }
    }

    struct FakeServices {
        apps: FakeApps,
    }

    impl QvmInputServices for FakeServices {
        fn applications(&self) -> &dyn QvmClientApplications {
            &self.apps
        }
        fn identity(&self, _slot: usize) -> Option<QvmClientIdentity> {
            None
        }
        fn live(&self, _identity: &QvmClientIdentity) -> bool {
            true
        }
        fn accepted(&self, _actor: &ActorId) -> Option<QvmClientCommand> {
            None
        }
        fn frame(&self) -> QvmFrameContext {
            QvmFrameContext {
                frame: 0,
                time: QvmSourceTime {
                    kind: QvmTimeKind::Milliseconds,
                    value: 0.0,
                },
                elapsed: QvmSourceTime {
                    kind: QvmTimeKind::Milliseconds,
                    value: 0.0,
                },
                phase: QvmFramePhase::FrameEntry,
            }
        }
        fn on_release(&self, _listener: QvmReleaseListener) -> QvmReleaseHandle {
            Box::new(|| {})
        }
    }

    fn artifact() -> QvmArtifact {
        QvmArtifact {
            module: ModuleIdentity {
                id: "test:qagame".to_string(),
                artifact_path: "test".to_string(),
                digest: "test".to_string(),
                revision: "1".to_string(),
            },
            role: GameRole::Qagame,
            abi_profile: None,
            image: QvmImage {
                allocated_data_length: 65536,
                ..Default::default()
            },
        }
    }

    fn server_state() -> Q3ServerState {
        let owner = IdentityOwner::create("q3-guest-test").unwrap();
        Q3ServerState::new(super::super::server_state::Q3ServerStateOptions {
            session: owner.session().clone(),
            settings: Q3HostSettings {
                game_type: 0,
                single_player: false,
                max_clients: 8,
                map_name: "q3dm1".to_string(),
                source_registry: None,
                source_archive: Vec::new(),
                cvars: Vec::new(),
            },
            now: Rc::new(|| 4242),
            print: Rc::new(|_| {}),
            register_server_cvars: Rc::new(|registry, max_clients, map_name| {
                registry.register("sv_maxclients", &max_clients.to_string(), 0).unwrap();
                registry.register("mapname", map_name, 0).unwrap();
            }),
        })
    }

    fn mounts() -> MountedContent {
        let plan = ResolvedMountPlan {
            id: create_mount_plan_id("guest-test", "1").unwrap(),
            mounts: Vec::new(),
            default_order: Vec::new(),
            prefix_orders: Vec::new(),
        };
        open_mount_plan(
            &plan,
            OpenMountOptions {
                pure: None,
                q3_restriction: None,
                links: Vec::new(),
                loose_comparison: None,
            },
        )
        .unwrap()
    }

    fn writable(name: &str) -> UserFileStore {
        let dir = std::env::temp_dir().join(format!("qa-guest-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        UserFileStore::new(dir)
    }

    fn common() -> Q3GuestCommon {
        Q3GuestCommon {
            milliseconds: Rc::new(|| 777),
            real_time: Rc::new(|_| 1234),
            commands: Q3GuestConsoleCommands {
                execute_now: Rc::new(|_| {}),
                insert: Rc::new(|_| {}),
                append: Rc::new(|_| {}),
            },
        }
    }

    struct Harness {
        guest: Q3QvmServerGame,
        owner: Rc<IdentityOwner>,
        output: Rc<FakeOutput>,
        actors: Rc<FakeActors>,
    }

    fn make_harness(name: &str, entity_text: &str, actor_name: &str) -> Harness {
        let owner = Rc::new(IdentityOwner::create(name).unwrap());
        let actors = Rc::new(FakeActors::new(actor_name));
        let output = Rc::new(FakeOutput::new());
        let guest = Q3QvmServerGame::new(Q3GuestRuntimeOptions {
            artifact: artifact(),
            state: server_state(),
            records: Q3GuestRecordHost {
                actors: actors.clone(),
                bodies: Rc::new(FakeBodies),
                scene: Rc::new(FakeScene),
                provider: ProviderId::new("q3", "guest-test"),
                collision: Rc::new(|_, _| {}),
                admit: None,
            },
            mounts: mounts(),
            writable: writable(name),
            max_clients: 8,
            dedicated: None,
            seed: 99,
            entity_text: entity_text.to_string(),
            common: common(),
            now: Rc::new(|| 4242),
            assert_current: Rc::new(|| {}),
            clip: Rc::new(|_, _| Rc::new(FakeClip)),
            before_disconnect: None,
            before_retire: None,
            source_restored: None,
            arsenal: None,
            client_changed: None,
            bot_command: None,
        })
        .unwrap();
        guest.state.cvars.borrow_mut().set("bot_enable", "0", true).unwrap();
        // Production VMs locate tables through `G_LOCATE_GAME_DATA`
        // during `GAME_INIT`; the test artifact cannot trap, so locate
        // directly with the same layout the records tests use.
        guest.game.data.locate(64, 16, 1024, 24576, 1024).unwrap();
        Harness {
            guest,
            owner,
            output,
            actors,
        }
    }

    fn stored() -> Q3StoredUserCommand {
        Q3StoredUserCommand {
            server_time: 100,
            angles: [10, 20, 30],
            forwardmove: 1,
            rightmove: 2,
            upmove: 3,
            buttons: 4,
            weapon: 5,
        }
    }

    #[test]
    fn constructor_registers_server_cvars() {
        let harness = make_harness("guest-ctor", "", "guest-ctor-actors");
        let registry = harness.guest.state.cvars.borrow();
        assert_eq!(registry.variable_string("sv_maxclients"), "8");
        assert_eq!(registry.variable_string("dedicated"), "1");
        assert_eq!(registry.variable_string("bot_enable"), "0");
        drop(registry);
        assert!(!harness.guest.is_retired());
        assert_eq!(harness.guest.time_milliseconds(), 4242);
        assert!(harness.guest.players().is_empty());
        assert!(!harness.guest.is_bot(&harness.owner.client(0, 1)));
    }

    #[test]
    fn initialize_runs_game_init() {
        let harness = make_harness("guest-init", "", "guest-init-actors");
        harness
            .guest
            .initialize(harness.output.clone(), Q3GuestInitialization::New)
            .unwrap();
        let calls = harness.guest.game.module.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].words[0], QvmGameExport::GAME_INIT);
        assert_eq!(calls[0].words[1], 4242);
        assert_eq!(calls[0].words[2], 99);
        // The constructor already refreshed server info, so `initialize`
        // publishes nothing new (donor `refreshServerInfo` returns null).
        assert!(harness.output.configstrings.borrow().is_empty());
    }

    #[test]
    fn client_lifecycle_admits_and_disconnects() {
        let harness = make_harness("guest-life", "", "guest-life-actors");
        harness
            .guest
            .initialize(harness.output.clone(), Q3GuestInitialization::New)
            .unwrap();
        let client = harness.owner.client(0, 1);
        let admission = harness.guest.connect(&client, "name=test").unwrap();
        let Q3ApplicationAdmission::Accepted { player } = admission else {
            panic!("expected admission");
        };
        assert_eq!(player.source_entity, 0);
        assert_eq!(harness.guest.state.get_userinfo(0).as_deref(), Some("name=test"));
        harness.guest.begin(&player, &stored()).unwrap();
        harness.guest.think(&player, &stored()).unwrap();
        harness.guest.userinfo(&player, "name=test2").unwrap();
        harness
            .guest
            .command_immediate(&player, &["say".to_string(), "hi".to_string()])
            .unwrap();
        harness.guest.command(&player, &["say".to_string()]).unwrap();
        assert_eq!(harness.guest.players(), vec![player.clone()]);
        assert_eq!(harness.guest.player(&client), Some(player.clone()));
        harness.guest.disconnect(&player).unwrap();
        assert!(harness.guest.players().is_empty());
        assert!(harness.guest.state.get_userinfo(0).is_none());
        let kinds: Vec<i32> = harness
            .guest
            .game
            .module
            .calls()
            .iter()
            .map(|call| call.words[0])
            .collect();
        assert!(kinds.contains(&QvmGameExport::GAME_CLIENT_CONNECT));
        assert!(kinds.contains(&QvmGameExport::GAME_CLIENT_BEGIN));
        assert!(kinds.contains(&QvmGameExport::GAME_CLIENT_THINK));
        assert!(kinds.contains(&QvmGameExport::GAME_CLIENT_USERINFO_CHANGED));
        let commands = harness.guest.game.module.commands();
        assert_eq!(commands.len(), 2);
        assert!(commands
            .iter()
            .all(|command| command.words[0] == QvmGameExport::GAME_CLIENT_COMMAND));
        assert!(kinds.contains(&QvmGameExport::GAME_CLIENT_DISCONNECT));
    }

    #[test]
    fn occupied_slot_rejects() {
        let harness = make_harness("guest-occupied", "", "guest-occupied-actors");
        harness
            .guest
            .initialize(harness.output.clone(), Q3GuestInitialization::New)
            .unwrap();
        let first = harness.owner.client(1, 1);
        let admission = harness.guest.connect(&first, "").unwrap();
        assert!(matches!(admission, Q3ApplicationAdmission::Accepted { .. }));
        let second = harness.owner.client(1, 2);
        let admission = harness.guest.connect(&second, "").unwrap();
        assert!(matches!(admission, Q3ApplicationAdmission::Rejected { .. }));
    }

    #[test]
    fn checkpoint_round_trip_restores_client() {
        let harness = make_harness("guest-save", "{ classname worldspawn }", "guest-save-actors");
        harness
            .guest
            .initialize(harness.output.clone(), Q3GuestInitialization::New)
            .unwrap();
        let client = harness.owner.client(2, 3);
        let admission = harness.guest.connect(&client, "name=saved").unwrap();
        let Q3ApplicationAdmission::Accepted { player } = admission else {
            panic!("expected admission");
        };
        harness.guest.begin(&player, &stored()).unwrap();
        let checkpoint = harness.guest.checkpoint().unwrap();
        assert_eq!(checkpoint.host_state.format, "q3:qagame-host");
        let saved = saved_q3_guest_clients(&checkpoint).unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].source_entity, 2);
        assert_eq!(saved[0].client.slot, 2);
        assert_eq!(saved[0].client.generation, 3);
        assert_eq!(saved[0].phase, Q3SavedClientPhase::Active);

        let fresh = make_harness("guest-restore", "{ classname worldspawn }", "guest-restore-actors");
        fresh
            .actors
            .allocate_at_source(&ProviderId::new("q3", "guest-test"), 2, "player");
        fresh
            .guest
            .restore_checkpoint(&checkpoint, {
                let owner = Rc::clone(&fresh.owner);
                move |saved| owner.client(saved.slot, saved.generation)
            })
            .unwrap();
        fresh.guest.complete_restore(fresh.output.clone());
        let players = fresh.guest.players();
        assert_eq!(players.len(), 1);
        assert_eq!(players[0].source_entity, 2);
        assert_eq!(players[0].client.slot(), 2);
        assert_eq!(fresh.guest.state.get_userinfo(2).as_deref(), Some("name=saved"));
    }

    #[test]
    fn map_transition_carries_clients() {
        let harness = make_harness("guest-map", "", "guest-map-actors");
        harness
            .guest
            .initialize(harness.output.clone(), Q3GuestInitialization::New)
            .unwrap();
        let client = harness.owner.client(0, 1);
        let admission = harness.guest.connect(&client, "name=carry").unwrap();
        assert!(matches!(admission, Q3ApplicationAdmission::Accepted { .. }));
        let transition = harness.guest.shutdown_for_map_change(false).unwrap();
        assert_eq!(transition.clients.len(), 1);
        assert_eq!(transition.clients[0].userinfo, "name=carry");
        assert!(harness.guest.is_retired());

        let next = make_harness("guest-map-next", "", "guest-map-next-actors");
        next.guest
            .initialize(
                next.output.clone(),
                Q3GuestInitialization::MapChange(transition.clients),
            )
            .unwrap();
        let admission = next.guest.reconnect(&client).unwrap();
        assert!(matches!(admission, Q3ApplicationAdmission::Accepted { .. }));
    }

    #[test]
    #[should_panic(expected = "Q3 player is no longer admitted")]
    fn stale_player_clone_rejected_after_readmission() {
        let harness = make_harness("guest-stale", "", "guest-stale-actors");
        harness
            .guest
            .initialize(harness.output.clone(), Q3GuestInitialization::New)
            .unwrap();
        let client = harness.owner.client(0, 1);
        let admission = harness.guest.connect(&client, "").unwrap();
        let Q3ApplicationAdmission::Accepted { player } = admission else {
            panic!("expected admission");
        };
        harness.guest.disconnect(&player).unwrap();
        let admission = harness.guest.connect(&client, "").unwrap();
        assert!(matches!(admission, Q3ApplicationAdmission::Accepted { .. }));
        harness.guest.begin(&player, &stored()).unwrap();
    }

    #[test]
    fn trap_dispatch_serves_common_and_disabled_bot() {
        let harness = make_harness("guest-trap", "", "guest-trap-actors");
        let memory = harness.guest.game.module.memory();
        memory.write_string(64, "hello", 16).unwrap();
        let trap = |code: i32, words: Vec<i32>| GameCall {
            kind: GameKind::Engine,
            role: GameRole::Qagame,
            code,
            words,
            guest: memory.clone(),
            abi_profile: GameAbi::Modern,
            command_arguments: None,
        };
        let printed = Rc::new(RefCell::new(Vec::<String>::new()));
        let _ = printed;
        assert_eq!(
            harness
                .guest
                .game
                .module
                .dispatch_host(&trap(QvmGameImport::G_MILLISECONDS, vec![2]))
                .unwrap(),
            777
        );
        assert_eq!(
            harness
                .guest
                .game
                .module
                .dispatch_host(&trap(QvmGameImport::G_PRINT, vec![0, 64]))
                .unwrap(),
            0
        );
        assert_eq!(
            harness
                .guest
                .game
                .module
                .dispatch_host(&trap(BOTLIB_SETUP, vec![200]))
                .unwrap(),
            0
        );
        assert_eq!(
            harness
                .guest
                .game
                .module
                .dispatch_host(&trap(BOTLIB_TEST, vec![208]))
                .unwrap(),
            0
        );
        assert_eq!(
            harness
                .guest
                .game
                .module
                .dispatch_host(&trap(BOTLIB_SHUTDOWN, vec![201]))
                .unwrap(),
            1
        );
        assert!(harness.guest.game.module.dispatch_host(&trap(999, vec![999])).is_err());
        assert!(harness
            .guest
            .game
            .module
            .dispatch_host(&trap(QvmGameImport::G_ERROR, vec![1, 64]))
            .is_err());
    }

    #[test]
    fn console_and_frame_and_configstrings() {
        let harness = make_harness("guest-frame", "", "guest-frame-actors");
        harness
            .guest
            .initialize(harness.output.clone(), Q3GuestInitialization::New)
            .unwrap();
        harness.guest.console_command(&["status".to_string()]).unwrap();
        harness.guest.run_frame(5000).unwrap();
        harness.guest.set_configstring(7, "seven");
        harness.guest.set_configstring(7, "seven");
        assert_eq!(harness.guest.state.configstring_get(7), "seven");
        let published: Vec<(i32, String)> = harness
            .output
            .configstrings
            .borrow()
            .iter()
            .filter(|(index, _)| *index == 7)
            .cloned()
            .collect();
        assert_eq!(published.len(), 1);
        harness.guest.shutdown().unwrap();
        assert!(harness.guest.is_retired());
    }

    #[test]
    fn bind_input_rebinds_and_shutdown_closes() {
        let harness = make_harness("guest-input", "", "guest-input-actors");
        let definition = || QvmInputDefinition {
            module: ModuleIdentity {
                id: "test:qagame".to_string(),
                artifact_path: "test".to_string(),
                digest: "test".to_string(),
                revision: "1".to_string(),
            },
            entity_stride: 560,
            client_stride: 560,
            client_pointer: 0,
            intermission: Vec::new(),
            movement_modes: None,
            entries: QvmInputEntries {
                client_think: 11,
                run_client: 12,
                client_spawn: 13,
                move_entry: 14,
                slice: 15,
            },
        };
        harness
            .guest
            .bind_input(definition(), Rc::new(FakeServices { apps: FakeApps }));
        harness
            .guest
            .bind_input(definition(), Rc::new(FakeServices { apps: FakeApps }));
        harness.guest.discard().unwrap();
        assert!(harness.guest.is_retired());
    }
}
