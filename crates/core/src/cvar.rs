//! Instance-owned cvar registry ported from `src/core/cvars/` (Q1/QW
//! `cvar.c`, Q2 `qcommon/cvar.c`, Q3 `cvar.c`) with policies from
//! `src/contracts/common.ts`.
//!
//! One registry per owner, parameterized by [`Dialect`]: exact vs
//! ASCII-folded name lookup, newest-first ordering, latch/archive/info
//! rules. Donor `print` calls become queued notifications plus a
//! structured [`CvarEffect`] queue; every effect carries the registry
//! session so owners drain effects into session context. VM mirrors,
//! aliases, value bindings, and save/restore are implemented here.
//!
//! Donor provenance:
//! `/home/buzzkill/Projects/quake-typescript/src/core/cvars/index.ts`,
//! `/home/buzzkill/Projects/quake-typescript/src/core/cvars/mirror.ts`.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicU64, Ordering};

use thiserror::Error;

use crate::cmd::{ascii_fold, source_command_text, Dialect};
use crate::identity::SessionId;
use crate::numeric::{native_atof, native_atoi};

/// Error for cvar operations.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CvarError {
    /// Command text must be source bytes.
    #[error(transparent)]
    Command(#[from] crate::cmd::CmdError),
    /// A value domain was violated; carries the donor message.
    #[error("{0}")]
    Domain(String),
}

/// Q3 ABI flag word. Q1 uses `ARCHIVE` and `SERVER_INFO` for declaration
/// booleans; the Archive/UserInfo/ServerInfo values match Q2.
pub mod flags {
    /// No flags.
    pub const NONE: u32 = 0;
    /// Archive to configuration.
    pub const ARCHIVE: u32 = 1;
    /// Send in client userinfo.
    pub const USER_INFO: u32 = 2;
    /// Send in server info.
    pub const SERVER_INFO: u32 = 4;
    /// Send in system info (Q3).
    pub const SYSTEM_INFO: u32 = 8;
    /// Write protected (Q3).
    pub const INIT: u32 = 16;
    /// Latched until restart (Q3).
    pub const LATCH: u32 = 32;
    /// Read only.
    pub const READ_ONLY: u32 = 64;
    /// Created by the user, not code.
    pub const USER_CREATED: u32 = 128;
    /// Temporary (Q3).
    pub const TEMPORARY: u32 = 256;
    /// Cheat protected.
    pub const CHEAT: u32 = 512;
    /// Survives `cvar_restart` (Q3).
    pub const NO_RESTART: u32 = 1024;
}

/// Quake II flag word.
pub mod q2_flags {
    /// No flags.
    pub const NONE: u32 = 0;
    /// Archive to configuration.
    pub const ARCHIVE: u32 = 1;
    /// Send in client userinfo.
    pub const USER_INFO: u32 = 2;
    /// Send in server info.
    pub const SERVER_INFO: u32 = 4;
    /// Write protected.
    pub const NO_SET: u32 = 8;
    /// Latched until the next game.
    pub const LATCH: u32 = 16;
    /// Cheat protected.
    pub const CHEAT: u32 = 32;
    /// Never sent to clients.
    pub const PRIVATE: u32 = 64;
    /// Read only.
    pub const READ_ONLY: u32 = 128;
    /// Modified since last check.
    pub const MODIFIED: u32 = 256;
    /// Created by the user, not code.
    pub const CUSTOM: u32 = 512;
    /// Ignored when set from the command line.
    pub const WEAK: u32 = 1024;
    /// Created by the game module.
    pub const GAME: u32 = 2048;
    /// Never archive.
    pub const NO_ARCHIVE: u32 = 4096;
    /// Added to `sv_paks` downloads.
    pub const FILES: u32 = 8192;
    /// Refresh needed.
    pub const REFRESH: u32 = 16384;
    /// Sound restart needed.
    pub const SOUND: u32 = 32768;
}

const Q2_NO_ARCHIVE: u32 =
    q2_flags::NO_SET | q2_flags::CHEAT | q2_flags::PRIVATE | q2_flags::READ_ONLY | q2_flags::NO_ARCHIVE;
const MAX_CVARS: usize = 1024;

/// Quake `Q_atof`: sign, `0x` hex, `'c'` character constant, or decimal
/// with an optional point. Stops at the first unrecognized byte.
#[must_use]
pub fn quake_atof(text: &str) -> f64 {
    let chars: Vec<char> = text.chars().collect();
    let at = |index: usize| chars.get(index).copied().unwrap_or('\0');
    let mut offset = 0;
    let mut sign = 1.0;
    if at(offset) == '-' {
        sign = -1.0;
        offset += 1;
    }
    if at(offset) == '0' && matches!(at(offset + 1), 'x' | 'X') {
        offset += 2;
        let mut value = 0.0;
        while offset < chars.len() {
            let digit = chars[offset];
            let digit = if digit.is_ascii_digit() {
                digit as i32 - 48
            } else if ('a'..='f').contains(&digit) {
                digit as i32 - 87
            } else if ('A'..='F').contains(&digit) {
                digit as i32 - 55
            } else {
                -1
            };
            offset += 1;
            if digit < 0 {
                break;
            }
            value = value * 16.0 + f64::from(digit);
        }
        return value * sign;
    }
    if at(offset) == '\'' {
        let code = if offset + 1 < chars.len() {
            chars[offset + 1] as u32
        } else {
            0
        };
        return sign * f64::from(code);
    }
    let mut value = 0.0;
    let mut decimal: i64 = -1;
    let mut total: i64 = 0;
    while offset < chars.len() {
        let byte = chars[offset] as u32;
        offset += 1;
        if byte == 46 {
            decimal = total;
            continue;
        }
        if !(48..=57).contains(&byte) {
            break;
        }
        value = value * 10.0 + f64::from(byte - 48);
        total += 1;
    }
    if decimal != -1 {
        while total > decimal {
            value /= 10.0;
            total -= 1;
        }
    }
    value * sign
}

/// C-locale `%f` rendering of a binary32 value: six decimals, ties to even.
/// With the integer shortcut, integral in-range values render plainly.
pub fn cvar_value_text(input: f64, integer_shortcut: bool) -> Result<String, CvarError> {
    let value = input as f32;
    if !value.is_finite() {
        return Err(CvarError::Domain("Cvar_SetValue requires a finite float".to_string()));
    }
    if integer_shortcut && (-2_147_483_648.0..2_147_483_648.0).contains(&value) && value == value.trunc() {
        return Ok(format!("{}", value as i32));
    }
    Ok(format!("{value:.6}"))
}

/// Info-string target for filtering rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfoTarget {
    /// Client userinfo string.
    ClientUserinfo,
    /// Server info string.
    ServerInfo,
}

/// Options for [`set_info_value`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InfoOptions {
    /// Console dialect.
    pub dialect: Dialect,
    /// Maximum info length including the terminator.
    pub maximum_length: usize,
    /// Which string is updated.
    pub target: InfoTarget,
    /// Whether the server keeps high bytes in server info (QW).
    pub server_high_characters: bool,
}

/// Set or remove a `\\key\\value` pair, following QW/Q2/Q3 `q_shared.c`.
/// Prints and keeps the input on rejected keys, values, or overflow.
pub fn set_info_value(
    input: &str,
    key_input: &str,
    value_input: &str,
    options: InfoOptions,
    print: &mut dyn FnMut(&str),
) -> Result<String, CvarError> {
    let info = source_command_text(input)?;
    let key = source_command_text(key_input)?;
    let value = source_command_text(value_input)?;
    let q3 = options.dialect == Dialect::Q3;
    let qw = options.dialect == Dialect::Q1Quakeworld;
    if info.chars().count() >= options.maximum_length {
        return Err(CvarError::Domain(
            "Info_SetValueForKey: oversize infostring".to_string(),
        ));
    }
    if key.contains('\\') || value.contains('\\') {
        print("Can't use keys or values with a \\\n");
        return Ok(info);
    }
    if (!qw && key.contains(';')) || (q3 && value.contains(';')) {
        print("Can't use keys or values with a semicolon\n");
        return Ok(info);
    }
    if key.contains('"') || value.contains('"') {
        print("Can't use keys or values with a \"\n");
        return Ok(info);
    }
    if qw && key.starts_with('*') {
        print("Can't set * keys\n");
        return Ok(info);
    }
    if !q3 && (key.chars().count() > 63 || value.chars().count() > 63) {
        print("Keys and values must be < 64 characters.\n");
        return Ok(info);
    }
    let chars: Vec<char> = info.chars().collect();
    let mut result = info.clone();
    let mut cursor = 0;
    while cursor < chars.len() {
        let start = cursor;
        if chars[cursor] == '\\' {
            cursor += 1;
        }
        let Some(separator) = chars[cursor..]
            .iter()
            .position(|c| *c == '\\')
            .map(|index| index + cursor)
        else {
            break;
        };
        let next = chars[separator + 1..]
            .iter()
            .position(|c| *c == '\\')
            .map(|index| index + separator + 1);
        let end = next.unwrap_or(chars.len());
        if chars[cursor..separator].iter().collect::<String>() == key {
            if qw
                && end > separator + 1
                && value.chars().count() - (end - separator - 1) + chars.len() > options.maximum_length
            {
                print("Info string length exceeded\n");
                return Ok(info);
            }
            result = chars[..start].iter().collect::<String>() + &chars[end..].iter().collect::<String>();
            break;
        }
        cursor = end;
    }
    if value.is_empty() {
        return Ok(result);
    }
    let mut pair = format!("\\{key}\\{value}");
    if q3 && pair.chars().count() >= options.maximum_length {
        pair = pair.chars().take(options.maximum_length - 1).collect();
    }
    if pair.chars().count() + result.chars().count() > options.maximum_length {
        print("Info string length exceeded\n");
        return Ok(result);
    }
    if !q3 {
        let mut filtered = String::new();
        for byte in pair.chars().map(|c| c as u32) {
            let mut byte = byte;
            if qw {
                let strip = if options.target == InfoTarget::ServerInfo {
                    !options.server_high_characters
                } else {
                    ascii_fold(&key) != "name"
                };
                if strip {
                    byte &= 127;
                    if !(32..=127).contains(&byte) {
                        continue;
                    }
                    if options.target == InfoTarget::ClientUserinfo
                        && ascii_fold(&key) == "team"
                        && (65..=90).contains(&byte)
                    {
                        byte += 32;
                    }
                }
                if byte > 13 {
                    filtered.push(char::from_u32(byte).unwrap_or('?'));
                }
            } else {
                byte &= 127;
                if (32..127).contains(&byte) {
                    filtered.push(char::from_u32(byte).unwrap_or('?'));
                }
            }
        }
        pair = filtered;
    }
    if pair.chars().count() + result.chars().count() == options.maximum_length {
        return Err(CvarError::Domain("Info string overflows source terminator".to_string()));
    }
    if q3 && options.maximum_length != 8192 {
        Ok(pair + &result)
    } else {
        Ok(result + &pair)
    }
}

/// Immutable snapshot of one variable.
#[derive(Debug, Clone, PartialEq)]
pub struct CvarSnapshot {
    /// Variable name in declaration case.
    pub name: String,
    /// Current value.
    pub value: String,
    /// Reset (default) value.
    pub reset_value: String,
    /// Latched value awaiting restart, if any.
    pub latched_value: Option<String>,
    /// Flag word.
    pub flags: u32,
    /// Modified since creation or last clear.
    pub modified: bool,
    /// Modification counter.
    pub modification_count: u32,
    /// Numeric value (`Q_atof`/`atof`, binary32).
    pub numeric_value: f32,
    /// Integer value (`atoi`).
    pub integer_value: i32,
}

/// Queued side effect for the owner to drain (broadcasts, info updates,
/// game-directory switches). Each effect carries the registry session so
/// the owner routes it into session context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CvarEffect {
    /// Client userinfo changed.
    Userinfo {
        /// Registry session that produced the effect.
        session: Option<SessionId>,
        /// Variable name.
        name: String,
        /// New value.
        value: String,
        /// Updated info string.
        info: String,
        /// `setinfo` command for the server, when connected.
        command: Option<String>,
    },
    /// Server info changed.
    ServerInfo {
        /// Registry session that produced the effect.
        session: Option<SessionId>,
        /// Variable name.
        name: String,
        /// New value.
        value: String,
        /// Updated info string.
        info: String,
    },
    /// Broadcast line for connected clients.
    Broadcast {
        /// Registry session that produced the effect.
        session: Option<SessionId>,
        /// Line text.
        text: String,
    },
    /// The `game` directory changed; the owner executes `autoexec`.
    GameDirectory {
        /// Registry session that produced the effect.
        session: Option<SessionId>,
        /// New directory.
        directory: String,
    },
}

/// Archive entry for configuration files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CvarArchiveEntry {
    /// Variable name.
    pub name: String,
    /// Archived value.
    pub value: String,
}

/// Help text for a cvar or command (donor `CommandDocumentation`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CvarDocumentation {
    /// One-line summary.
    pub summary: String,
    /// Usage line.
    pub usage: String,
    /// Example invocations.
    pub examples: Vec<String>,
    /// Allowed values, when the value is an enum.
    pub allowed_values: Option<Vec<String>>,
}

/// Live value binding: validates writes before they enter registry state
/// and observes every committed value (donor `CvarValueBinding`).
pub trait CvarValueBinding {
    /// Return an explanation to reject `value`, or `None` to accept it.
    fn validate(&self, value: &str) -> Option<String>;
    /// Observe a committed value.
    fn changed(&mut self, value: &str);
}

type ValidateFn = dyn Fn(&str) -> Option<String>;
type ChangedFn = dyn FnMut(&str);

/// Closure-backed [`CvarValueBinding`].
pub struct FnValueBinding {
    validate: Box<ValidateFn>,
    changed: Box<ChangedFn>,
}

impl FnValueBinding {
    /// Build a binding from a validator and a change observer.
    pub fn new(validate: impl Fn(&str) -> Option<String> + 'static, changed: impl FnMut(&str) + 'static) -> Self {
        Self {
            validate: Box::new(validate),
            changed: Box::new(changed),
        }
    }
}

impl CvarValueBinding for FnValueBinding {
    fn validate(&self, value: &str) -> Option<String> {
        (self.validate)(value)
    }

    fn changed(&mut self, value: &str) {
        (self.changed)(value);
    }
}

/// Token identifying one installed value binding; releases it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BindingToken(u64);

/// Name alias projecting a canonical variable, optionally through a value
/// conversion (donor `CvarAlias`).
pub struct CvarAlias {
    /// Alias name.
    pub name: String,
    /// Canonical target variable.
    pub target: String,
    /// Help text served when the alias itself is undocumented.
    pub documentation: CvarDocumentation,
    /// Value conversion between alias and canonical text.
    pub conversion: CvarAliasConversion,
}

type AliasWriteFn = dyn Fn(&str) -> Result<String, String>;

/// Alias value conversion.
pub enum CvarAliasConversion {
    /// Alias reads and writes canonical text unchanged.
    Identity,
    /// Converted alias: `read` projects canonical text, `write` maps alias
    /// text back (or rejects with a message).
    Converted {
        /// Project canonical text to alias text.
        read: Box<dyn Fn(&str) -> String>,
        /// Map alias text to canonical text.
        write: Box<AliasWriteFn>,
    },
}

impl CvarAlias {
    /// Project canonical text to alias text.
    #[must_use]
    pub fn read(&self, value: &str) -> String {
        match &self.conversion {
            CvarAliasConversion::Identity => value.to_string(),
            CvarAliasConversion::Converted { read, .. } => read(value),
        }
    }
}

/// One saved variable in registry-index order (donor `CvarSnapshot` row).
#[derive(Debug, Clone, PartialEq)]
pub struct SavedCvarState {
    /// Variable name in declaration case.
    pub name: String,
    /// Current value.
    pub value: String,
    /// Reset (default) value.
    pub reset_value: String,
    /// Latched value awaiting restart, if any.
    pub latched_value: Option<String>,
    /// Flag word.
    pub flags: u32,
    /// Modified since creation or last clear.
    pub modified: bool,
    /// Modification counter.
    pub modification_count: u32,
    /// Numeric value.
    pub numeric_value: f32,
    /// Integer value.
    pub integer_value: i32,
}

/// Typed capture of registry state (donor `captureWorldTransferState` and
/// `captureSaveState` payloads).
#[derive(Debug, Clone, PartialEq)]
pub struct CvarSaveState {
    /// Registry dialect; restore rejects a mismatch.
    pub dialect: Dialect,
    /// Variable rows in registry-index order (`None` marks holes left by
    /// `reset_all` and converted-alias VM handles).
    pub variables: Vec<Option<SavedCvarState>>,
    /// Variable names newest-first.
    pub order: Vec<String>,
    /// Accumulated modification flags.
    pub changed_flags: u32,
    /// Cheat permission fallback.
    pub cheats_enabled: bool,
    /// Server activity.
    pub server_active: bool,
    /// Client connection.
    pub client_connected: bool,
    /// High-character support for QW server info.
    pub high_characters: bool,
    /// Propagated QW client info.
    pub client_info: String,
    /// Propagated QW server info.
    pub server_info: String,
    /// Userinfo-modified flag.
    pub userinfo_dirty: bool,
    /// Console-created variable keys.
    pub console_variables: Vec<String>,
    /// Converted-alias VM handles as `(handle, alias name)` pairs.
    pub alias_handles: Vec<(usize, String)>,
}

/// Validated pending restore (donor `prepareRegistryRestore` applier).
pub struct PendingCvarRestore {
    variables: HashMap<String, CvarState>,
    indexes: Vec<Option<String>>,
    first_keys_newest: Vec<String>,
    changed_flags: u32,
    cheats_enabled: bool,
    server_active: bool,
    client_connected: bool,
    high_characters: bool,
    client_info: String,
    server_info: String,
    userinfo_dirty: bool,
    console_variables: HashSet<String>,
    alias_handles: HashMap<usize, String>,
}

impl PendingCvarRestore {
    /// Apply the restore, replacing registry state and seeding `effects`.
    pub fn apply(self, registry: &mut CvarRegistry, effects: Vec<CvarEffect>) {
        registry.variables = self.variables;
        registry.alias_handles = self.alias_handles;
        registry.indexes = self.indexes;
        registry.order = self.first_keys_newest.into_iter().rev().collect();
        registry.effects = effects;
        registry.changed_flags = self.changed_flags;
        registry.cheats_enabled = self.cheats_enabled;
        registry.server_active = self.server_active;
        registry.client_connected = self.client_connected;
        registry.high_characters = self.high_characters;
        registry.client_info = self.client_info;
        registry.server_info = self.server_info;
        registry.userinfo_dirty = self.userinfo_dirty;
        registry.console_variables = self.console_variables;
        let bound: Vec<(String, String)> = registry
            .value_bindings
            .keys()
            .filter_map(|key| {
                registry
                    .variables
                    .get(key)
                    .map(|state| (key.clone(), state.value.clone()))
            })
            .collect();
        for (key, value) in bound {
            if let Some((_, binding)) = registry.value_bindings.get_mut(&key) {
                binding.changed(&value);
            }
        }
    }
}

/// Q3 VM mirror of one cvar (donor `VmCvar`).
pub trait VmCvar {
    /// Mirrored value text.
    fn value(&self) -> &str;
    /// Mirrored numeric value.
    fn numeric_value(&self) -> f32;
    /// Mirrored integer value.
    fn integer_value(&self) -> i32;
    /// Mirrored modification counter.
    fn modification_count(&self) -> u32;
    /// Bind to `name`, registering the default, then refresh.
    fn register(&mut self, name: &str, default_value: &str, flags: u32) -> Result<(), CvarError>;
    /// Refresh from the registry when the counter moved.
    fn update(&mut self) -> Result<(), CvarError>;
    /// Write the VM-local integer slot (does not touch the registry).
    fn write_integer(&mut self, value: i64) -> Result<(), CvarError>;
}

type CommandExistsCallback = dyn Fn(&str) -> bool;

/// Which `set*` command family flags a variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]

pub enum SetCommandKind {
    /// `seta`: archive.
    Archive,
    /// `setu`: userinfo.
    Userinfo,
    /// `sets`: serverinfo.
    Serverinfo,
}

#[derive(Debug, Clone)]
struct CvarState {
    index: usize,
    name: String,
    value: String,
    reset_value: String,
    latched_value: Option<String>,
    flags: u32,
    modified: bool,
    modification_count: u32,
    numeric_value: f32,
    integer_value: i32,
}

impl CvarState {
    fn snapshot(&self) -> CvarSnapshot {
        CvarSnapshot {
            name: self.name.clone(),
            value: self.value.clone(),
            reset_value: self.reset_value.clone(),
            latched_value: self.latched_value.clone(),
            flags: self.flags,
            modified: self.modified,
            modification_count: self.modification_count,
            numeric_value: self.numeric_value,
            integer_value: self.integer_value,
        }
    }
}

/// Instance-owned cvar registry for one dialect.
#[derive(Clone)]
pub struct CvarRegistry {
    registry_id: u64,
    dialect: Dialect,
    session: Option<SessionId>,
    variables: HashMap<String, CvarState>,
    indexes: Vec<Option<String>>,
    order: Vec<String>,
    changed_flags: u32,
    cheats_enabled: bool,
    cheats_override: Option<bool>,
    server_active: bool,
    client_connected: bool,
    high_characters: bool,
    client_info: String,
    server_info: String,
    userinfo_dirty: bool,
    console_variables: HashSet<String>,
    info_targets: Vec<InfoTarget>,
    command_exists: Option<Rc<CommandExistsCallback>>,
    notifications: Vec<String>,
    effects: Vec<CvarEffect>,
    documents: HashMap<String, CvarDocumentation>,
    value_bindings: HashMap<String, (BindingToken, Box<dyn CvarValueBinding>)>,
    next_binding_token: u64,
    aliases: HashMap<String, CvarAlias>,
    alias_handles: HashMap<usize, String>,
}

static NEXT_REGISTRY_ID: AtomicU64 = AtomicU64::new(1);

impl CvarRegistry {
    /// Create an empty registry for a dialect.
    #[must_use]
    pub fn new(dialect: Dialect) -> Self {
        Self::with_session(dialect, None)
    }

    /// Create an empty registry bound to a session; effects carry it.
    #[must_use]
    pub fn with_session(dialect: Dialect, session: Option<SessionId>) -> Self {
        Self {
            registry_id: NEXT_REGISTRY_ID.fetch_add(1, Ordering::Relaxed),
            dialect,
            session,
            variables: HashMap::new(),
            indexes: Vec::new(),
            order: Vec::new(),
            changed_flags: 0,
            cheats_enabled: true,
            cheats_override: None,
            server_active: false,
            client_connected: false,
            high_characters: false,
            client_info: String::new(),
            server_info: String::new(),
            userinfo_dirty: false,
            console_variables: HashSet::new(),
            info_targets: vec![InfoTarget::ClientUserinfo, InfoTarget::ServerInfo],
            command_exists: None,
            notifications: Vec::new(),
            effects: Vec::new(),
            documents: HashMap::new(),
            value_bindings: HashMap::new(),
            next_binding_token: 1,
            aliases: HashMap::new(),
            alias_handles: HashMap::new(),
        }
    }

    /// Registry dialect.
    #[must_use]
    pub fn dialect(&self) -> Dialect {
        self.dialect
    }

    /// Bound session, carried by every queued effect.
    #[must_use]
    pub fn session(&self) -> Option<&SessionId> {
        self.session.as_ref()
    }

    /// Bind the registry to a session.
    pub fn set_session(&mut self, session: SessionId) {
        self.session = Some(session);
    }

    pub(crate) fn registry_id(&self) -> u64 {
        self.registry_id
    }

    /// Install the `commandExists` callback used by Q1 registration.
    pub fn set_command_exists(&mut self, callback: Rc<CommandExistsCallback>) {
        self.command_exists = Some(callback);
    }

    /// Apply archived values as `seta` commands (donor `applyArchive`).
    pub fn apply_archive(&mut self, entries: &[CvarArchiveEntry]) -> Result<(), CvarError> {
        for entry in entries {
            self.set_command_flags(&entry.name, &entry.value, SetCommandKind::Archive)?;
        }
        Ok(())
    }

    /// Override cheat permission (a local game consults its live authority).
    pub fn set_cheats_override(&mut self, allowed: Option<bool>) {
        self.cheats_override = allowed;
    }

    /// Take queued notification lines.
    #[must_use]
    pub fn take_notifications(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notifications)
    }

    /// Take queued effects.
    #[must_use]
    pub fn take_effects(&mut self) -> Vec<CvarEffect> {
        std::mem::take(&mut self.effects)
    }

    fn print(&mut self, text: &str) {
        self.notifications.push(text.to_string());
    }

    fn key(&self, name: &str) -> String {
        if self.dialect == Dialect::Q3 {
            ascii_fold(name)
        } else {
            name.to_string()
        }
    }

    fn numbers(&self, value: &str) -> (f32, i32) {
        let numeric = if self.dialect.is_q1() {
            quake_atof(value) as f32
        } else {
            native_atof(value).unwrap_or(0.0) as f32
        };
        (numeric, native_atoi(value).unwrap_or(0))
    }

    /// Find a variable snapshot by name, projecting aliases.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<CvarSnapshot> {
        let key = self.key(&source_command_text(name).ok()?);
        if let Some(alias) = self.aliases.get(&key) {
            let target = self.key(&alias.target);
            return self
                .variables
                .get(&target)
                .map(|state| self.project_alias(alias, state));
        }
        self.variables.get(&key).map(CvarState::snapshot)
    }

    fn project_alias(&self, alias: &CvarAlias, state: &CvarState) -> CvarSnapshot {
        let value = alias.read(&state.value);
        let (numeric_value, integer_value) = self.numbers(&value);
        CvarSnapshot {
            name: alias.name.clone(),
            value,
            reset_value: alias.read(&state.reset_value),
            latched_value: state.latched_value.as_deref().map(|latched| alias.read(latched)),
            flags: state.flags,
            modified: state.modified,
            modification_count: state.modification_count,
            numeric_value,
            integer_value,
        }
    }

    /// Canonical variable behind a name (the name itself when unaliased).
    #[must_use]
    pub fn canonical_name(&self, name: &str) -> String {
        source_command_text(name)
            .ok()
            .and_then(|clean| self.aliases.get(&self.key(&clean)).map(|alias| alias.target.clone()))
            .unwrap_or_else(|| name.to_string())
    }

    fn reject_alias_info_flags(&self, name: &str, flag_word: u32) -> Result<(), CvarError> {
        let mut mask = flags::USER_INFO | flags::SERVER_INFO;
        if self.dialect == Dialect::Q3 {
            mask |= flags::SYSTEM_INFO;
        }
        if (flag_word & mask) != 0 {
            return Err(CvarError::Domain(format!(
                "Cvar alias {name} requires an explicit protocol info-key mapping"
            )));
        }
        Ok(())
    }

    fn alias_write(&mut self, alias_name: &str, value: &str) -> Option<String> {
        let key = self.key(alias_name);
        let alias = self.aliases.get(&key)?;
        if matches!(alias.conversion, CvarAliasConversion::Identity) {
            return Some(value.to_string());
        }
        let CvarAliasConversion::Converted { write, .. } = &alias.conversion else {
            return Some(value.to_string());
        };
        let converted = write(value);
        match converted {
            Ok(text) => Some(text),
            Err(message) => {
                let name = self
                    .aliases
                    .get(&key)
                    .map_or_else(String::new, |alias| alias.name.clone());
                let notice = format!("{name}: {message}\n");
                self.print(&notice);
                None
            }
        }
    }

    /// Current value text, or empty when unregistered.
    #[must_use]
    pub fn variable_string(&self, name: &str) -> String {
        self.get(name).map_or_else(String::new, |snapshot| snapshot.value)
    }

    /// Current numeric value, or zero when unregistered.
    #[must_use]
    pub fn variable_value(&self, name: &str) -> f32 {
        self.get(name).map_or(0.0, |snapshot| snapshot.numeric_value)
    }

    /// Whether the name was created from the console.
    #[must_use]
    pub fn is_console_created(&self, name: &str) -> bool {
        let canonical = self.canonical_name(name);
        source_command_text(&canonical).is_ok_and(|text| self.console_variables.contains(&self.key(&text)))
    }

    /// Snapshots newest-first, optionally filtered by flag mask; alias
    /// projections follow the canonical variables.
    #[must_use]
    pub fn snapshots(&self, flags: u32) -> Vec<CvarSnapshot> {
        let mut values: Vec<CvarSnapshot> = self
            .order
            .iter()
            .rev()
            .filter_map(|key| self.variables.get(key))
            .filter(|state| flags == 0 || state.flags & flags != 0)
            .map(CvarState::snapshot)
            .collect();
        let mut names: Vec<String> = self.aliases.values().map(|alias| alias.name.clone()).collect();
        names.sort();
        for name in names {
            if let Some(projected) = self.get(&name) {
                if flags == 0 || (projected.flags & flags) != 0 {
                    values.push(projected);
                }
            }
        }
        values
    }

    /// Complete a partial name against registered variables.
    pub fn complete(&self, partial_input: &str) -> Result<Option<String>, CvarError> {
        let partial = source_command_text(partial_input)?;
        if partial.is_empty() {
            return Ok(None);
        }
        if self.dialect != Dialect::Q1Netquake {
            if let Some(found) = self.get(&partial) {
                return Ok(Some(found.name));
            }
        }
        let prefix = self.key(&partial);
        Ok(self
            .snapshots(0)
            .into_iter()
            .find(|state| self.key(&state.name).starts_with(&prefix))
            .map(|state| state.name))
    }

    fn allow_cheats(&self) -> bool {
        if let Some(allowed) = self.cheats_override {
            return allowed;
        }
        self.get("sv_cheats")
            .map_or(self.cheats_enabled, |cheats| cheats.integer_value != 0)
    }

    fn apply_value(&mut self, key: &str, value: String, mark: bool) {
        let (numeric_value, integer_value) = self.numbers(&value);
        if let Some(state) = self.variables.get_mut(key) {
            if mark {
                state.modified = true;
                state.modification_count += 1;
            }
            state.value = value.clone();
            state.numeric_value = numeric_value;
            state.integer_value = integer_value;
        }
        if let Some((_, binding)) = self.value_bindings.get_mut(key) {
            binding.changed(&value);
        }
    }

    fn binding_changed(&mut self, key: &str, value: &str) {
        if let Some((_, binding)) = self.value_bindings.get_mut(key) {
            binding.changed(value);
        }
    }

    fn valid_bound_value(&mut self, name: &str, value: &str) -> bool {
        let key = self.key(name);
        let error = self
            .value_bindings
            .get(&key)
            .and_then(|(_, binding)| binding.validate(value));
        if let Some(error) = error {
            let notice = format!("{name}: {error}\n");
            self.print(&notice);
            return false;
        }
        true
    }

    fn valid_info(text: &str) -> bool {
        !text.chars().any(|c| c == '\\' || c == '"' || c == ';')
    }

    /// Register a variable. Re-registration merges flags per dialect and
    /// never replaces a live value.
    pub fn register(
        &mut self,
        name_input: &str,
        default_input: &str,
        flag_input: u32,
    ) -> Result<Option<CvarSnapshot>, CvarError> {
        let mut name = source_command_text(name_input)?;
        // A donor declaration of an alias must not replace the canonical default or policy.
        if self.aliases.contains_key(&self.key(&name)) {
            self.reject_alias_info_flags(&name.clone(), flag_input)?;
            return Ok(self.get(&name));
        }
        let default_value = source_command_text(default_input)?;
        if !self.valid_bound_value(&name.clone(), &default_value) {
            return Ok(self.get(&name));
        }
        if self.dialect == Dialect::Q3 && !Self::valid_info(&name) {
            self.print(&format!("invalid cvar name string: {name}\n"));
            name = "BADNAME".to_string();
        }
        if self.dialect.is_q2() && (flag_input & 6) != 0 && !Self::valid_info(&name) {
            self.print("invalid info cvar name\n");
            return Ok(None);
        }
        let key = self.key(&name);
        if let Some(existing) = self.variables.get(&key).map(CvarState::snapshot) {
            if self.dialect.is_q1() {
                if self.console_variables.remove(&key) {
                    let flags = flag_input;
                    if let Some(state) = self.variables.get_mut(&key) {
                        state.reset_value = default_value.clone();
                        state.flags |= flags;
                    }
                    if self.dialect == Dialect::Q1Quakeworld {
                        let value = default_value.clone();
                        self.propagate(&key, &value, true)?;
                    }
                } else {
                    self.print(&format!("Can't register variable {name}, allready defined\n"));
                }
                return Ok(Some(existing));
            }
            if self.dialect == Dialect::Q3 {
                if (existing.flags & flags::USER_CREATED) != 0
                    && (flag_input & flags::USER_CREATED) == 0
                    && !default_value.is_empty()
                {
                    if let Some(state) = self.variables.get_mut(&key) {
                        state.flags &= !flags::USER_CREATED;
                        state.reset_value = default_value.clone();
                    }
                    self.changed_flags |= flag_input;
                }
                if existing.reset_value.is_empty() {
                    if let Some(state) = self.variables.get_mut(&key) {
                        state.reset_value = default_value.clone();
                    }
                }
            }
            if self.dialect.is_q2() && (existing.flags & q2_flags::CUSTOM) != 0 && (flag_input & q2_flags::CUSTOM) == 0
            {
                let needs_reset = (flag_input & (q2_flags::READ_ONLY | q2_flags::NO_SET)) != 0
                    || ((flag_input & q2_flags::CHEAT) != 0 && !self.allow_cheats())
                    || ((flag_input & (q2_flags::USER_INFO | q2_flags::SERVER_INFO)) != 0
                        && !Self::valid_info(&existing.value));
                if let Some(state) = self.variables.get_mut(&key) {
                    state.reset_value = default_value.clone();
                    state.flags &= !q2_flags::CUSTOM;
                }
                if needs_reset {
                    let default = default_value.clone();
                    self.set(&name.clone(), &default, true)?;
                }
            }
            if let Some(state) = self.variables.get_mut(&key) {
                state.flags |= flag_input;
                if self.dialect.is_q2() && (flag_input & Q2_NO_ARCHIVE) != 0 {
                    state.flags &= !q2_flags::ARCHIVE;
                }
            }
            if self.dialect == Dialect::Q3 {
                let latched = self.variables.get(&key).and_then(|state| state.latched_value.clone());
                if let Some(latched) = latched {
                    if let Some(state) = self.variables.get_mut(&key) {
                        state.latched_value = None;
                    }
                    self.set(&name.clone(), &latched, true)?;
                }
            }
            return Ok(self.variables.get(&key).map(CvarState::snapshot));
        }
        if self.dialect.is_q1() && self.command_exists.as_ref().is_some_and(|exists| exists(&name)) {
            self.print(&format!("Cvar_RegisterVariable: {name} is a command\n"));
            return Ok(None);
        }
        if self.dialect.is_q2() && (flag_input & 6) != 0 && !Self::valid_info(&default_value) {
            self.print("invalid info cvar value\n");
            return Ok(None);
        }
        if self.dialect == Dialect::Q3 && self.indexes.len() == MAX_CVARS {
            return Err(CvarError::Domain("MAX_CVARS".to_string()));
        }
        let (numeric_value, integer_value) = self.numbers(&default_value);
        let state = CvarState {
            index: self.indexes.len(),
            name: name.clone(),
            value: default_value.clone(),
            reset_value: default_value.clone(),
            latched_value: None,
            flags: flag_input,
            modified: true,
            modification_count: 1,
            numeric_value,
            integer_value,
        };
        self.variables.insert(key.clone(), state);
        self.indexes.push(Some(key.clone()));
        self.order.push(key.clone());
        if self.dialect == Dialect::Q1Quakeworld {
            self.propagate(&key, &default_value, true)?;
        }
        Ok(self.variables.get(&key).map(CvarState::snapshot))
    }

    /// Set a variable. Q3/Q2 create unknown variables; Q1 reports them.
    pub fn set(&mut self, name_input: &str, value_input: &str, force: bool) -> Result<Option<CvarSnapshot>, CvarError> {
        let mut name = source_command_text(name_input)?;
        if self.aliases.contains_key(&self.key(&name)) {
            let target = self.canonical_name(&name);
            let clean = source_command_text(value_input)?;
            let Some(converted) = self.alias_write(&name.clone(), &clean) else {
                return Ok(None);
            };
            if self.set(&target, &converted, force)?.is_none() {
                return Ok(None);
            }
            return Ok(self.get(&name));
        }
        if self.dialect == Dialect::Q3 && !Self::valid_info(&name) {
            self.print(&format!("invalid cvar name string: {name}\n"));
            name = "BADNAME".to_string();
        }
        let value = source_command_text(value_input)?;
        let key = self.key(&name);
        if !self.valid_bound_value(&name.clone(), &value) {
            return Ok(None);
        }
        if !self.variables.contains_key(&key) {
            if self.dialect.is_q1() {
                self.print(&format!("Cvar_Set: variable {name} not found\n"));
                return Ok(None);
            }
            let flags = if self.dialect == Dialect::Q3 && !force {
                flags::USER_CREATED
            } else {
                0
            };
            return self.register(&name, &value, flags);
        }
        if self.dialect.is_q1() {
            let changed = self.variables.get(&key).is_some_and(|state| state.value != value);
            self.propagate(&key, &value, changed)?;
            self.apply_value(&key, value, false);
            return Ok(self.variables.get(&key).map(CvarState::snapshot));
        }
        if self.dialect.is_q2() {
            let info = self.variables.get(&key).is_some_and(|state| (state.flags & 6) != 0);
            if info && !Self::valid_info(&value) {
                self.print("invalid info cvar value\n");
                return Ok(self.variables.get(&key).map(CvarState::snapshot));
            }
            if !force {
                let state = self.variables.get(&key).map(CvarState::snapshot).unwrap();
                if (state.flags & q2_flags::READ_ONLY) != 0 {
                    self.print(&format!("{name} is read only.\n"));
                    return Ok(Some(state));
                }
                if (state.flags & q2_flags::CHEAT) != 0 && !self.allow_cheats() {
                    self.print(&format!("{name} is cheat protected.\n"));
                    return Ok(Some(state));
                }
                if (state.flags & q2_flags::NO_SET) != 0 {
                    self.print(&format!("{name} is write protected.\n"));
                    return Ok(Some(state));
                }
                if (state.flags & q2_flags::LATCH) != 0 {
                    let current = state.latched_value.clone().unwrap_or(state.value.clone());
                    if value == current {
                        return Ok(Some(state));
                    }
                    if let Some(stored) = self.variables.get_mut(&key) {
                        stored.latched_value = None;
                    }
                    if self.server_active {
                        self.print(&format!("{name} will be changed for next game.\n"));
                        if let Some(stored) = self.variables.get_mut(&key) {
                            stored.latched_value = Some(value);
                        }
                    } else {
                        self.apply_value(&key, value, false);
                        self.game_directory(&key);
                    }
                    return Ok(self.variables.get(&key).map(CvarState::snapshot));
                }
            } else if let Some(stored) = self.variables.get_mut(&key) {
                stored.latched_value = None;
            }
        } else {
            let state = self.variables.get(&key).map(CvarState::snapshot).unwrap();
            if value == state.value {
                // Q3 checks equality before forced writes clear an outstanding latch.
                self.binding_changed(&key.clone(), &value);
                return Ok(Some(state));
            }
            self.changed_flags |= state.flags;
            if !force {
                if (state.flags & flags::READ_ONLY) != 0 {
                    self.print(&format!("{name} is read only.\n"));
                    return Ok(Some(state));
                }
                if (state.flags & flags::INIT) != 0 {
                    self.print(&format!("{name} is write protected.\n"));
                    return Ok(Some(state));
                }
                if (state.flags & flags::LATCH) != 0 {
                    if state.latched_value.as_deref() == Some(value.as_str()) {
                        return Ok(Some(state));
                    }
                    self.print(&format!("{name} will be changed upon restarting.\n"));
                    if let Some(stored) = self.variables.get_mut(&key) {
                        stored.latched_value = Some(value);
                        stored.modified = true;
                        stored.modification_count += 1;
                    }
                    return Ok(self.variables.get(&key).map(CvarState::snapshot));
                }
                if (state.flags & flags::CHEAT) != 0 && !self.allow_cheats() {
                    self.print(&format!("{name} is cheat protected.\n"));
                    return Ok(Some(state));
                }
            } else if let Some(stored) = self.variables.get_mut(&key) {
                stored.latched_value = None;
            }
        }
        let changed = self.variables.get(&key).is_some_and(|state| state.value != value);
        if changed {
            self.apply_value(&key, value.clone(), true);
            if self.dialect.is_q2()
                && self
                    .variables
                    .get(&key)
                    .is_some_and(|state| (state.flags & q2_flags::USER_INFO) != 0)
            {
                self.userinfo_dirty = true;
            }
        } else {
            self.binding_changed(&key.clone(), &value);
        }
        Ok(self.variables.get(&key).map(CvarState::snapshot))
    }

    /// Console `set`: like [`CvarRegistry::set`], but a Q2 write of the
    /// current value clears the latch instead.
    pub fn set_console(&mut self, name: &str, value: &str) -> Result<Option<CvarSnapshot>, CvarError> {
        let clean_name = source_command_text(name)?;
        if self.aliases.contains_key(&self.key(&clean_name)) {
            let target = self.canonical_name(&clean_name);
            let clean = source_command_text(value)?;
            let Some(converted) = self.alias_write(&clean_name.clone(), &clean) else {
                return Ok(None);
            };
            if self.set_console(&target, &converted)?.is_none() {
                return Ok(None);
            }
            return Ok(self.get(&clean_name));
        }
        let key = self.key(&clean_name);
        if self.dialect.is_q2() && self.variables.get(&key).is_some_and(|state| state.value == value) {
            if let Some(state) = self.variables.get_mut(&key) {
                state.latched_value = None;
            }
            self.binding_changed(&key.clone(), value);
            return Ok(self.variables.get(&key).map(CvarState::snapshot));
        }
        self.set(name, value, false)
    }

    /// `seta`/`setu`/`sets`: set a value and OR in the command flag,
    /// creating console/user variables as needed.
    pub fn set_command_flags(&mut self, name: &str, value: &str, kind: SetCommandKind) -> Result<(), CvarError> {
        let q2 = self.dialect.is_q2();
        let flag = match kind {
            SetCommandKind::Archive => {
                if q2 {
                    q2_flags::ARCHIVE
                } else {
                    flags::ARCHIVE
                }
            }
            SetCommandKind::Userinfo => {
                if q2 {
                    q2_flags::USER_INFO
                } else {
                    flags::USER_INFO
                }
            }
            SetCommandKind::Serverinfo => {
                if q2 {
                    q2_flags::SERVER_INFO
                } else {
                    flags::SERVER_INFO
                }
            }
        };
        let clean_name = source_command_text(name)?;
        if self.aliases.contains_key(&self.key(&clean_name)) {
            if kind != SetCommandKind::Archive {
                let notice = format!("Cvar alias {clean_name} requires an explicit protocol info-key mapping\n");
                self.print(&notice);
                return Ok(());
            }
            let target = self.canonical_name(&clean_name);
            let clean_value = source_command_text(value)?;
            if let Some(converted) = self.alias_write(&clean_name.clone(), &clean_value) {
                self.set_command_flags(&target, &converted, kind)?;
            }
            return Ok(());
        }
        let clean_value = source_command_text(value)?;
        if !self.valid_bound_value(&clean_name.clone(), &clean_value) {
            return Ok(());
        }
        let key = self.key(&clean_name);
        let previous_flags = self.variables.get(&key).map_or(0, |state| state.flags);
        if kind != SetCommandKind::Archive
            && (!Self::valid_info(&clean_name)
                || !Self::valid_info(&clean_value)
                || (q2 && (clean_name.len() >= 64 || clean_value.len() >= 64)))
        {
            self.print("invalid info cvar name or value\n");
            return Ok(());
        }
        if !self.variables.contains_key(&key) {
            let create_flags = flag
                | if q2 {
                    q2_flags::CUSTOM
                } else if self.dialect == Dialect::Q3 {
                    flags::USER_CREATED
                } else {
                    0
                };
            let created = self.register(&clean_name, &clean_value, create_flags)?;
            if let Some(created) = created {
                if self.dialect.is_q1() {
                    let created_key = self.key(&created.name);
                    self.console_variables.insert(created_key);
                }
            } else {
                return Ok(());
            }
        } else {
            self.set_console(&clean_name, &clean_value)?;
            let retained_bad = self
                .variables
                .get(&key)
                .is_some_and(|state| !Self::valid_info(&state.value) || (q2 && state.value.len() >= 64));
            if kind != SetCommandKind::Archive && retained_bad {
                self.print("invalid retained info cvar value\n");
                return Ok(());
            }
            if kind != SetCommandKind::Archive && q2 {
                if let Some(state) = self.variables.get_mut(&key) {
                    state.flags &= !(q2_flags::USER_INFO | q2_flags::SERVER_INFO);
                }
            }
            if kind == SetCommandKind::Archive || !q2 || (previous_flags & Q2_NO_ARCHIVE) == 0 {
                if let Some(state) = self.variables.get_mut(&key) {
                    state.flags |= flag;
                }
            }
        }
        if q2
            && self
                .variables
                .get(&key)
                .is_some_and(|state| (state.flags & Q2_NO_ARCHIVE) != 0)
        {
            if let Some(state) = self.variables.get_mut(&key) {
                state.flags &= !q2_flags::ARCHIVE;
            }
        }
        if kind != SetCommandKind::Archive {
            if q2 {
                let current = self.variables.get(&key).map_or(0, |state| state.flags);
                if ((previous_flags | current) & q2_flags::USER_INFO) != 0 {
                    self.userinfo_dirty = true;
                }
            } else {
                let value = self
                    .variables
                    .get(&key)
                    .map_or_else(String::new, |state| state.value.clone());
                self.propagate(&key, &value, true)?;
            }
        }
        Ok(())
    }

    /// Quake II `Cvar_FullSet`: replace the value and the whole flag word.
    pub fn full_set(&mut self, name: &str, value: &str, flag_word: u32) -> Result<Option<CvarSnapshot>, CvarError> {
        if !self.dialect.is_q2() {
            return Err(CvarError::Domain("Cvar_FullSet belongs to Quake II".to_string()));
        }
        let clean_name = source_command_text(name)?;
        if self.aliases.contains_key(&self.key(&clean_name)) {
            self.reject_alias_info_flags(&clean_name.clone(), flag_word)?;
            let target = self.canonical_name(&clean_name);
            let clean_value = source_command_text(value)?;
            let Some(converted) = self.alias_write(&clean_name.clone(), &clean_value) else {
                return Ok(None);
            };
            if self.full_set(&target, &converted, flag_word)?.is_none() {
                return Ok(None);
            }
            return Ok(self.get(&clean_name));
        }
        let key = self.key(&clean_name);
        let clean_value = source_command_text(value)?;
        if !self.valid_bound_value(&clean_name.clone(), &clean_value) {
            return Ok(self.variables.get(&key).map(CvarState::snapshot));
        }
        if !self.variables.contains_key(&key) {
            return self.register(name, &clean_value, flag_word);
        }
        if self
            .variables
            .get(&key)
            .is_some_and(|state| (state.flags & q2_flags::USER_INFO) != 0)
        {
            self.userinfo_dirty = true;
        }
        self.apply_value(&key, clean_value, true);
        if let Some(state) = self.variables.get_mut(&key) {
            state.flags = flag_word;
        }
        Ok(self.variables.get(&key).map(CvarState::snapshot))
    }

    /// `Cvar_SetValue`: format a float and set it (forced on Q3).
    pub fn set_value(&mut self, name: &str, value: f64) -> Result<Option<CvarSnapshot>, CvarError> {
        let text = cvar_value_text(value, !self.dialect.is_q1())?;
        if text.len() >= 32 {
            if self.dialect.is_q1() {
                return Err(CvarError::Domain("Cvar_SetValue overflows source val[32]".to_string()));
            }
            self.print(&format!("Com_sprintf: overflow of {} in 32\n", text.len()));
        }
        let truncated: String = text.chars().take(31).collect();
        self.set(name, &truncated, self.dialect == Dialect::Q3)
    }

    /// Defer a value on a registered variable without changing flags.
    pub fn stage(&mut self, name: &str, input: &str) -> Result<CvarSnapshot, CvarError> {
        let clean_name = source_command_text(name)?;
        if self.aliases.contains_key(&self.key(&clean_name)) {
            let target = self.canonical_name(&clean_name);
            let clean = source_command_text(input)?;
            if let Some(converted) = self.alias_write(&clean_name.clone(), &clean) {
                self.stage(&target, &converted)?;
            }
            return self
                .get(&clean_name)
                .ok_or_else(|| CvarError::Domain(format!("Cannot stage an unregistered cvar {name}")));
        }
        let key = self.key(&clean_name);
        let value = source_command_text(input)?;
        let Some(state) = self.variables.get(&key).map(CvarState::snapshot) else {
            return Err(CvarError::Domain(format!("Cannot stage an unregistered cvar {name}")));
        };
        if !self.valid_bound_value(&clean_name.clone(), &value) {
            return Ok(state);
        }
        if self.dialect == Dialect::Q3 && (state.flags & (flags::READ_ONLY | flags::INIT)) != 0
            || self.dialect.is_q2() && (state.flags & q2_flags::NO_SET) != 0
        {
            return Err(CvarError::Domain(format!("Cannot stage a protected cvar {name}")));
        }
        if self.dialect.is_q2() && (state.flags & 6) != 0 && !Self::valid_info(&value) {
            return Err(CvarError::Domain(format!("Invalid staged info cvar {name}")));
        }
        let pending = if value == state.value { None } else { Some(value) };
        if self.variables.get(&key).and_then(|stored| stored.latched_value.clone()) != pending {
            if let Some(stored) = self.variables.get_mut(&key) {
                stored.latched_value = pending;
                stored.modified = true;
                stored.modification_count += 1;
            }
            if self.dialect == Dialect::Q3 {
                self.changed_flags |= state.flags;
            }
        }
        Ok(self.variables.get(&key).map(CvarState::snapshot).unwrap())
    }

    /// Apply latched values (all, or one name).
    pub fn apply_latched(&mut self, name: Option<&str>) -> Result<Vec<CvarSnapshot>, CvarError> {
        let filter = name
            .map(|text| source_command_text(text).map(|clean| self.key(&self.canonical_name(&clean))))
            .transpose()?;
        let mut changed = Vec::new();
        let order: Vec<String> = self.order.iter().rev().cloned().collect();
        for key in order {
            if filter.as_ref().is_some_and(|wanted| *wanted != key) {
                continue;
            }
            let Some(latched) = self.variables.get(&key).and_then(|state| state.latched_value.clone()) else {
                continue;
            };
            if let Some(state) = self.variables.get_mut(&key) {
                state.latched_value = None;
            }
            let mark = self.dialect == Dialect::Q3;
            self.apply_value(&key, latched, mark);
            if self.dialect.is_q2() {
                self.game_directory(&key);
            }
            if let Some(state) = self.variables.get(&key) {
                changed.push(state.snapshot());
            }
        }
        Ok(changed)
    }

    /// Reset one variable to its default.
    pub fn reset(&mut self, name: &str, force: bool) -> Result<Option<CvarSnapshot>, CvarError> {
        let Some(state) = self.get(name) else { return Ok(None) };
        self.set(name, &state.reset_value.clone(), force)
    }

    /// Console `reset`: like `set_console` to the default, skipping
    /// protected variables when resetting all.
    pub fn reset_console(&mut self, name: &str, all: bool) -> Result<(), CvarError> {
        let Some(state) = self.get(name) else { return Ok(()) };
        if all && (name == "game" || name == "fs_game") {
            return Ok(());
        }
        if all
            && (self.dialect.is_q2() && (state.flags & (q2_flags::NO_SET | q2_flags::READ_ONLY)) != 0
                || self.dialect == Dialect::Q3
                    && (state.flags & (flags::READ_ONLY | flags::INIT | flags::NO_RESTART)) != 0)
        {
            return Ok(());
        }
        self.set_console(name, &state.reset_value.clone())?;
        Ok(())
    }

    /// Quake III `cvar_restart`: reset every variable and delete
    /// user-created ones.
    pub fn reset_all(&mut self) -> Result<(), CvarError> {
        if self.dialect != Dialect::Q3 {
            return Err(CvarError::Domain("cvar_restart belongs to Quake III".to_string()));
        }
        let order: Vec<String> = self.order.iter().rev().cloned().collect();
        for key in order {
            let Some(state) = self.variables.get(&key).map(CvarState::snapshot) else {
                continue;
            };
            if (state.flags & (flags::READ_ONLY | flags::INIT | flags::NO_RESTART)) != 0 {
                continue;
            }
            if (state.flags & flags::USER_CREATED) != 0 {
                if let Some(stored) = self.variables.remove(&key) {
                    if let Some(slot) = self.indexes.get_mut(stored.index) {
                        *slot = None;
                    }
                }
                self.documents.remove(&key);
                self.order.retain(|entry| *entry != key);
            } else {
                self.set(&state.name.clone(), &state.reset_value.clone(), true)?;
            }
        }
        Ok(())
    }

    /// Enable or disable cheats. Disabling on Q3 resets cheat variables.
    pub fn set_cheats_enabled(&mut self, enabled: bool) -> Result<(), CvarError> {
        self.cheats_enabled = enabled;
        if enabled || self.dialect != Dialect::Q3 {
            return Ok(());
        }
        let order: Vec<String> = self.order.iter().rev().cloned().collect();
        for key in order {
            let cheat = self
                .variables
                .get(&key)
                .is_some_and(|state| (state.flags & flags::CHEAT) != 0);
            if !cheat {
                continue;
            }
            if let Some(state) = self.variables.get_mut(&key) {
                state.latched_value = None;
            }
            let snapshot = self.variables.get(&key).map(CvarState::snapshot).unwrap();
            self.set(&snapshot.name.clone(), &snapshot.reset_value.clone(), true)?;
        }
        Ok(())
    }

    /// Build the info string for a flag mask, newest-first.
    pub fn info_string(&mut self, flag_mask: u32, maximum_length: Option<usize>) -> Result<String, CvarError> {
        let maximum_length = maximum_length.unwrap_or(if self.dialect == Dialect::Q3 { 1024 } else { 512 });
        let target = if (flag_mask & flags::USER_INFO) != 0 {
            InfoTarget::ClientUserinfo
        } else {
            InfoTarget::ServerInfo
        };
        let mut info = String::new();
        let order: Vec<String> = self.order.iter().rev().cloned().collect();
        for key in order {
            let Some(state) = self.variables.get(&key).map(CvarState::snapshot) else {
                continue;
            };
            if (state.flags & flag_mask) == 0 {
                continue;
            }
            if self.dialect.is_q2() && (state.flags & q2_flags::PRIVATE) != 0 {
                continue;
            }
            let options = InfoOptions {
                dialect: self.dialect,
                maximum_length,
                target,
                server_high_characters: self.high_characters,
            };
            let mut notifications = Vec::new();
            let updated = set_info_value(&info, &state.name, &state.value, options, &mut |text| {
                notifications.push(text.to_string());
            })?;
            self.notifications.extend(notifications);
            info = updated;
        }
        Ok(info)
    }

    /// Last propagated info string for a target (QW).
    #[must_use]
    pub fn propagated_info(&self, target: InfoTarget) -> &str {
        match target {
            InfoTarget::ClientUserinfo => &self.client_info,
            InfoTarget::ServerInfo => &self.server_info,
        }
    }

    /// Whether userinfo changed since the last clear.
    #[must_use]
    pub fn userinfo_modified(&self) -> bool {
        self.userinfo_dirty
    }

    /// Clear the userinfo-modified flag.
    pub fn clear_userinfo_modified(&mut self) {
        self.userinfo_dirty = false;
    }

    /// Take and clear accumulated modification flags.
    #[must_use]
    pub fn take_modified_flags(&mut self) -> u32 {
        let flags = self.changed_flags;
        self.changed_flags = 0;
        flags
    }

    /// OR flags into the modification accumulator.
    pub fn mark_modified_flags(&mut self, flag_mask: u32) {
        self.changed_flags |= flag_mask;
    }

    /// Clear flags from the modification accumulator.
    pub fn clear_modified_flags(&mut self, flag_mask: u32) {
        self.changed_flags &= !flag_mask;
    }

    /// OR flags into a variable.
    pub fn add_flags(&mut self, name: &str, flag_mask: u32) -> Result<(), CvarError> {
        let clean = source_command_text(name)?;
        if self.aliases.contains_key(&self.key(&clean)) {
            self.reject_alias_info_flags(&clean.clone(), flag_mask)?;
        }
        let key = self.key(&self.canonical_name(&clean));
        if let Some(state) = self.variables.get_mut(&key) {
            state.flags |= flag_mask;
        }
        Ok(())
    }

    /// Clear a variable's modified bit.
    pub fn clear_modified(&mut self, name: &str) -> Result<(), CvarError> {
        let clean = source_command_text(name)?;
        let key = self.key(&self.canonical_name(&clean));
        if let Some(state) = self.variables.get_mut(&key) {
            state.modified = false;
        }
        Ok(())
    }

    /// Note server activity (drives Q2 latch and Q1 broadcast behavior).
    pub fn set_server_active(&mut self, active: bool) {
        self.server_active = active;
    }

    /// Note client connection (drives QW `setinfo` commands).
    pub fn set_client_connected(&mut self, connected: bool) {
        self.client_connected = connected;
    }

    /// Note high-character support for QW server info.
    pub fn set_server_high_characters(&mut self, enabled: bool) {
        self.high_characters = enabled;
    }

    fn archive_states(&self, include: &dyn Fn(&str) -> bool) -> Vec<CvarSnapshot> {
        self.order
            .iter()
            .rev()
            .filter_map(|key| self.variables.get(key))
            .filter(|state| include(&state.name))
            .filter(|state| !(self.dialect.is_q2() && (state.flags & Q2_NO_ARCHIVE) != 0))
            .filter(|state| (state.flags & flags::ARCHIVE) != 0)
            .filter(|state| !(self.dialect == Dialect::Q3 && ascii_fold(&state.name) == "cl_cdkey"))
            .map(CvarState::snapshot)
            .collect()
    }

    fn archive_value(&self, state: &CvarSnapshot) -> String {
        if self.dialect == Dialect::Q3 {
            state.latched_value.clone().unwrap_or_else(|| state.value.clone())
        } else {
            state.value.clone()
        }
    }

    /// Archive entries for configuration files.
    #[must_use]
    pub fn archive_entries(&self, include: &dyn Fn(&str) -> bool) -> Vec<CvarArchiveEntry> {
        self.archive_states(include)
            .iter()
            .map(|state| CvarArchiveEntry {
                name: state.name.clone(),
                value: self.archive_value(state),
            })
            .collect()
    }

    /// Archive lines (`seta`/`set`/bare per dialect and origin).
    #[must_use]
    pub fn archive_commands(&self, include: &dyn Fn(&str) -> bool) -> Vec<String> {
        self.archive_states(include)
            .iter()
            .map(|state| {
                let custom = self.dialect.is_q2() && (state.flags & q2_flags::CUSTOM) != 0;
                let prefix =
                    if self.dialect == Dialect::Q3 || self.console_variables.contains(&self.key(&state.name)) || custom
                    {
                        "seta "
                    } else if self.dialect.is_q2() {
                        "set "
                    } else {
                        ""
                    };
                format!("{prefix}{} \"{}\"", state.name, self.archive_value(state))
            })
            .collect()
    }

    /// Write archive lines, truncating non-Q1 lines at 1023 bytes.
    pub fn write_variables(&mut self, include: &dyn Fn(&str) -> bool, write: &mut dyn FnMut(&str)) {
        for command in self.archive_commands(include) {
            let line = format!("{command}\n");
            if !self.dialect.is_q1() && line.len() >= 1024 {
                self.print(&format!("Com_sprintf: overflow of {} in 1024\n", line.len()));
            }
            if self.dialect.is_q1() {
                write(&line);
            } else {
                write(&line.chars().take(1023).collect::<String>());
            }
        }
    }

    fn game_directory(&mut self, key: &str) {
        let session = self.session.clone();
        if let Some(state) = self.variables.get(key).map(CvarState::snapshot) {
            if state.name == "game" {
                self.effects.push(CvarEffect::GameDirectory {
                    session,
                    directory: state.value,
                });
            }
        }
    }

    fn propagate(&mut self, key: &str, value: &str, changed: bool) -> Result<(), CvarError> {
        let Some(state) = self.variables.get(key).map(CvarState::snapshot) else {
            return Ok(());
        };
        if self.dialect == Dialect::Q1Netquake {
            if (state.flags & flags::SERVER_INFO) != 0 && changed && self.server_active {
                self.effects.push(CvarEffect::Broadcast {
                    session: self.session.clone(),
                    text: format!("\"{}\" changed to \"{value}\"\n", state.name),
                });
            }
            return Ok(());
        }
        if self.dialect != Dialect::Q1Quakeworld {
            return Ok(());
        }
        for target in self.info_targets.clone() {
            let flag = if target == InfoTarget::ClientUserinfo {
                flags::USER_INFO
            } else {
                flags::SERVER_INFO
            };
            if (state.flags & flag) == 0 {
                continue;
            }
            let maximum_length = if target == InfoTarget::ClientUserinfo { 196 } else { 512 };
            let options = InfoOptions {
                dialect: self.dialect,
                maximum_length,
                target,
                server_high_characters: self.high_characters,
            };
            let mut notifications = Vec::new();
            let current = self.propagated_info(target).to_string();
            let updated = set_info_value(&current, &state.name, value, options, &mut |text| {
                notifications.push(text.to_string());
            })?;
            self.notifications.extend(notifications);
            if target == InfoTarget::ClientUserinfo {
                self.client_info = updated.clone();
                self.effects.push(CvarEffect::Userinfo {
                    session: self.session.clone(),
                    name: state.name.clone(),
                    value: value.to_string(),
                    info: updated,
                    command: self
                        .client_connected
                        .then(|| format!("setinfo \"{}\" \"{value}\"\n", state.name)),
                });
            } else {
                self.server_info = updated.clone();
                self.effects.push(CvarEffect::ServerInfo {
                    session: self.session.clone(),
                    name: state.name.clone(),
                    value: value.to_string(),
                    info: updated,
                });
            }
        }
        Ok(())
    }

    /// Register a name alias projecting a canonical variable.
    pub fn register_alias(&mut self, alias: CvarAlias) -> Result<(), CvarError> {
        let clean_name = source_command_text(&alias.name)?;
        let clean_target = source_command_text(&alias.target)?;
        let key = self.key(&clean_name);
        let target = self.key(&clean_target);
        if key == target || self.aliases.contains_key(&target) {
            return Err(CvarError::Domain(format!(
                "Cvar alias {} must target a canonical variable, not an alias or itself",
                alias.name
            )));
        }
        if self.variables.contains_key(&key)
            || self.aliases.contains_key(&key)
            || self.command_exists.as_ref().is_some_and(|exists| exists(&clean_name))
        {
            return Err(CvarError::Domain(format!(
                "Cvar alias {} is already declared",
                alias.name
            )));
        }
        let Some(target_flags) = self.variables.get(&target).map(|state| state.flags) else {
            return Err(CvarError::Domain(format!(
                "Cvar alias {} target {} is not registered",
                alias.name, alias.target
            )));
        };
        self.reject_alias_info_flags(&clean_name, target_flags)?;
        self.aliases.insert(key, alias);
        Ok(())
    }

    /// Bind a live value binding to a canonical variable; returns a token
    /// that releases exactly this binding.
    pub fn bind_value(&mut self, name: &str, binding: Box<dyn CvarValueBinding>) -> Result<BindingToken, CvarError> {
        let clean = source_command_text(name)?;
        if self.aliases.contains_key(&self.key(&clean)) {
            return Err(CvarError::Domain(format!(
                "Bind the canonical cvar {} instead of alias {name}",
                self.canonical_name(&clean)
            )));
        }
        let key = self.key(&clean);
        let Some(state) = self.variables.get(&key).map(CvarState::snapshot) else {
            return Err(CvarError::Domain(format!("Cannot bind unregistered cvar {name}")));
        };
        if self.value_bindings.contains_key(&key) {
            return Err(CvarError::Domain(format!("Cvar {name} already has a value binding")));
        }
        let mut candidates = vec![state.value.clone(), state.reset_value.clone()];
        candidates.extend(state.latched_value.clone());
        for value in &candidates {
            if let Some(error) = binding.validate(value) {
                return Err(CvarError::Domain(format!("{name}: {error}")));
            }
        }
        let token = BindingToken(self.next_binding_token);
        self.next_binding_token += 1;
        self.value_bindings.insert(key, (token, binding));
        Ok(token)
    }

    /// Release the binding installed under `token`.
    pub fn release_value_binding(&mut self, token: BindingToken) -> bool {
        let key = self
            .value_bindings
            .iter()
            .find(|(_, (installed, _))| *installed == token)
            .map(|(key, _)| key.clone());
        key.is_some_and(|key| self.value_bindings.remove(&key).is_some())
    }

    /// Attach help text to a registered variable or alias.
    pub fn document(&mut self, name: &str, documentation: CvarDocumentation) -> Result<(), CvarError> {
        let clean = source_command_text(name)?;
        if self.get(&clean).is_none() {
            return Err(CvarError::Domain(format!("Cannot document unregistered cvar {name}")));
        }
        self.documents.insert(self.key(&clean), documentation);
        Ok(())
    }

    /// Help text for a variable: its own document, else the alias document.
    #[must_use]
    pub fn documentation(&self, name: &str) -> Option<CvarDocumentation> {
        let clean = source_command_text(name).ok()?;
        self.get(&clean)?;
        let key = self.key(&clean);
        self.documents
            .get(&key)
            .cloned()
            .or_else(|| self.aliases.get(&key).map(|alias| alias.documentation.clone()))
    }

    /// Apply archived entries as `seta` writes.
    pub fn apply_archive(&mut self, entries: &[CvarArchiveEntry]) -> Result<(), CvarError> {
        for entry in entries {
            self.set_command_flags(&entry.name.clone(), &entry.value.clone(), SetCommandKind::Archive)?;
        }
        Ok(())
    }

    /// Registry-index length backing VM handles.
    #[must_use]
    pub fn index_count(&self) -> usize {
        self.indexes.len()
    }

    /// Bind a Q3 VM handle, registering the variable when needed.
    /// Converted aliases get their own handle slot.
    pub fn bind_vm(&mut self, name: &str, default_value: &str, flag_word: u32) -> Result<usize, CvarError> {
        if self.dialect != Dialect::Q3 {
            return Err(CvarError::Domain("VM cvar handles belong to Quake III".to_string()));
        }
        let clean = source_command_text(name)?;
        if let Some(alias) = self.aliases.get(&self.key(&clean)) {
            if matches!(alias.conversion, CvarAliasConversion::Converted { .. }) {
                self.reject_alias_info_flags(&clean.clone(), flag_word)?;
                let wanted = self.key(&clean);
                if let Some((&handle, _)) = self
                    .alias_handles
                    .iter()
                    .find(|(_, existing)| self.key(existing) == wanted)
                {
                    return Ok(handle);
                }
                if self.indexes.len() == MAX_CVARS {
                    return Err(CvarError::Domain("MAX_CVARS".to_string()));
                }
                let handle = self.indexes.len();
                self.indexes.push(None);
                self.alias_handles.insert(handle, alias.name.clone());
                return Ok(handle);
            }
        }
        let registered = self.register(&clean, default_value, flag_word)?;
        let key = registered
            .map(|snapshot| self.key(&self.canonical_name(&snapshot.name)))
            .unwrap_or_default();
        self.variables
            .get(&key)
            .map(|state| state.index)
            .ok_or_else(|| CvarError::Domain("VM cvar registration failed".to_string()))
    }

    /// Read the snapshot behind a VM handle.
    pub fn read_vm(&self, handle: usize) -> Result<Option<CvarSnapshot>, CvarError> {
        if handle >= self.indexes.len() {
            return Err(CvarError::Domain("Cvar_Update: handle out of range".to_string()));
        }
        if let Some(alias) = self.alias_handles.get(&handle) {
            return Ok(self.get(alias));
        }
        Ok(self.indexes[handle]
            .as_ref()
            .and_then(|key| self.variables.get(key))
            .map(CvarState::snapshot))
    }

    /// Capture a typed save image; fails while effects are undrained.
    pub fn capture_save_state(&self) -> Result<CvarSaveState, CvarError> {
        if !self.effects.is_empty() {
            return Err(CvarError::Domain("Cvar save requires drained effects".to_string()));
        }
        Ok(self.snapshot_registry_state())
    }

    /// Capture a world-transfer image without consuming pending effects.
    #[must_use]
    pub fn capture_world_transfer_state(&self) -> CvarSaveState {
        self.snapshot_registry_state()
    }

    fn snapshot_registry_state(&self) -> CvarSaveState {
        CvarSaveState {
            dialect: self.dialect,
            variables: self
                .indexes
                .iter()
                .map(|slot| {
                    slot.as_ref().and_then(|key| {
                        self.variables.get(key).map(|state| SavedCvarState {
                            name: state.name.clone(),
                            value: state.value.clone(),
                            reset_value: state.reset_value.clone(),
                            latched_value: state.latched_value.clone(),
                            flags: state.flags,
                            modified: state.modified,
                            modification_count: state.modification_count,
                            numeric_value: state.numeric_value,
                            integer_value: state.integer_value,
                        })
                    })
                })
                .collect(),
            order: self
                .order
                .iter()
                .rev()
                .filter_map(|key| self.variables.get(key))
                .map(|state| state.name.clone())
                .collect(),
            changed_flags: self.changed_flags,
            cheats_enabled: self.cheats_enabled,
            server_active: self.server_active,
            client_connected: self.client_connected,
            high_characters: self.high_characters,
            client_info: self.client_info.clone(),
            server_info: self.server_info.clone(),
            userinfo_dirty: self.userinfo_dirty,
            console_variables: {
                let mut names: Vec<String> = self.console_variables.iter().cloned().collect();
                names.sort();
                names
            },
            alias_handles: {
                let mut handles: Vec<(usize, String)> = self
                    .alias_handles
                    .iter()
                    .map(|(&handle, name)| (handle, name.clone()))
                    .collect();
                handles.sort();
                handles
            },
        }
    }

    /// Restore a save image captured from the same dialect.
    pub fn restore_save_state(&mut self, state: &CvarSaveState) -> Result<(), CvarError> {
        if state.dialect != self.dialect {
            return Err(CvarError::Domain("Cvar save dialect mismatch".to_string()));
        }
        let pending = self.prepare_registry_restore(state)?;
        pending.apply(self, Vec::new());
        Ok(())
    }

    /// Capture a Q1 QuakeC save image.
    pub fn capture_quake_c_state(&self) -> Result<CvarSaveState, CvarError> {
        if !self.dialect.is_q1() {
            return Err(CvarError::Domain("QC cvar save requires a Q1 registry".to_string()));
        }
        self.capture_save_state()
    }

    /// Restore a Q1 QuakeC save image.
    pub fn restore_quake_c_state(&mut self, state: &CvarSaveState) -> Result<(), CvarError> {
        if !self.dialect.is_q1() {
            return Err(CvarError::Domain("QC cvar restore requires a Q1 registry".to_string()));
        }
        if state.dialect != self.dialect {
            return Err(CvarError::Domain("Cvar save dialect mismatch".to_string()));
        }
        let pending = self.prepare_registry_restore(state)?;
        pending.apply(self, Vec::new());
        Ok(())
    }

    /// Validate a save image against the live aliases and bindings,
    /// returning a pending restore for two-phase publication.
    pub fn prepare_registry_restore(&self, state: &CvarSaveState) -> Result<PendingCvarRestore, CvarError> {
        let mut variables: HashMap<String, CvarState> = HashMap::new();
        let mut indexes: Vec<Option<String>> = Vec::with_capacity(state.variables.len());
        for (index, saved) in state.variables.iter().enumerate() {
            let Some(saved) = saved else {
                indexes.push(None);
                continue;
            };
            let key = self.key(&saved.name);
            if self.aliases.contains_key(&key) {
                return Err(CvarError::Domain(format!(
                    "saved cvar {} conflicts with an alias",
                    saved.name
                )));
            }
            if variables.contains_key(&key) {
                return Err(CvarError::Domain("duplicate cvar".to_string()));
            }
            let stored = CvarState {
                index,
                name: saved.name.clone(),
                value: saved.value.clone(),
                reset_value: saved.reset_value.clone(),
                latched_value: saved.latched_value.clone(),
                flags: saved.flags,
                modified: saved.modified,
                modification_count: saved.modification_count,
                numeric_value: saved.numeric_value,
                integer_value: saved.integer_value,
            };
            indexes.push(Some(key.clone()));
            variables.insert(key, stored);
        }
        let order_keys: Vec<String> = state.order.iter().map(|name| self.key(name)).collect();
        let unique: HashSet<&String> = order_keys.iter().collect();
        if unique.len() != variables.len() || order_keys.len() != variables.len() {
            return Err(CvarError::Domain("invalid cvar order".to_string()));
        }
        for key in &order_keys {
            if !variables.contains_key(key) {
                return Err(CvarError::Domain("unknown ordered cvar".to_string()));
            }
        }
        let console_variables: HashSet<String> = state.console_variables.iter().cloned().collect();
        if console_variables.len() != state.console_variables.len() {
            return Err(CvarError::Domain("duplicate console variable".to_string()));
        }
        let mut alias_handles: HashMap<usize, String> = HashMap::new();
        for (handle, name) in &state.alias_handles {
            let alias = self.aliases.get(&self.key(name));
            let valid = *handle < indexes.len()
                && indexes[*handle].is_none()
                && !alias_handles.contains_key(handle)
                && alias.is_some_and(|alias| {
                    matches!(alias.conversion, CvarAliasConversion::Converted { .. })
                        && variables.contains_key(&self.key(&alias.target))
                });
            if !valid {
                return Err(CvarError::Domain("invalid cvar alias handle".to_string()));
            }
            alias_handles.insert(*handle, name.clone());
        }
        for (key, (_, binding)) in &self.value_bindings {
            let Some(stored) = variables.get(key) else {
                return Err(CvarError::Domain(format!("missing bound cvar {key}")));
            };
            let mut candidates = vec![stored.value.clone(), stored.reset_value.clone()];
            candidates.extend(stored.latched_value.clone());
            for value in &candidates {
                if let Some(error) = binding.validate(value) {
                    return Err(CvarError::Domain(format!("{key}: {error}")));
                }
            }
        }
        Ok(PendingCvarRestore {
            variables,
            indexes,
            first_keys_newest: order_keys,
            changed_flags: state.changed_flags,
            cheats_enabled: state.cheats_enabled,
            server_active: state.server_active,
            client_connected: state.client_connected,
            high_characters: state.high_characters,
            client_info: state.client_info.clone(),
            server_info: state.server_info.clone(),
            userinfo_dirty: state.userinfo_dirty,
            console_variables,
            alias_handles,
        })
    }
}

/// Registry-backed Q3 VM mirror (donor `RegistryVmCvar`).
pub struct RegistryVmCvar {
    registry: Rc<RefCell<CvarRegistry>>,
    handle: Option<usize>,
    text: String,
    numeric: f32,
    integer: i32,
    count: i64,
}

impl RegistryVmCvar {
    /// Attach a VM mirror to a Q3 registry.
    pub fn attach(registry: Rc<RefCell<CvarRegistry>>) -> Result<Self, CvarError> {
        if registry.borrow().dialect() != Dialect::Q3 {
            return Err(CvarError::Domain("VM cvar mirrors belong to Quake III".to_string()));
        }
        Ok(Self {
            registry,
            handle: None,
            text: String::new(),
            numeric: 0.0,
            integer: 0,
            count: 0,
        })
    }

    /// Attach and register in one step (donor `registerVm`).
    pub fn registered(
        registry: Rc<RefCell<CvarRegistry>>,
        name: &str,
        default_value: &str,
        flags: u32,
    ) -> Result<Self, CvarError> {
        let mut vm = Self::attach(registry)?;
        vm.register(name, default_value, flags)?;
        Ok(vm)
    }
}

impl VmCvar for RegistryVmCvar {
    fn value(&self) -> &str {
        &self.text
    }

    fn numeric_value(&self) -> f32 {
        self.numeric
    }

    fn integer_value(&self) -> i32 {
        self.integer
    }

    fn modification_count(&self) -> u32 {
        self.count.max(0) as u32
    }

    fn register(&mut self, name: &str, default_value: &str, flags: u32) -> Result<(), CvarError> {
        let handle = self.registry.borrow_mut().bind_vm(name, default_value, flags)?;
        self.handle = Some(handle);
        self.count = -1;
        self.update()
    }

    fn update(&mut self) -> Result<(), CvarError> {
        let Some(handle) = self.handle else {
            return Ok(());
        };
        let source = self.registry.borrow().read_vm(handle)?;
        let Some(source) = source else {
            return Ok(());
        };
        if i64::from(source.modification_count) == self.count {
            return Ok(());
        }
        self.count = i64::from(source.modification_count);
        if source.value.len() > 255 {
            return Err(CvarError::Domain(
                "Cvar_Update: value exceeds MAX_CVAR_VALUE_STRING".to_string(),
            ));
        }
        self.text = source.value;
        self.numeric = source.numeric_value;
        self.integer = source.integer_value;
        Ok(())
    }

    fn write_integer(&mut self, value: i64) -> Result<(), CvarError> {
        if !(-(1 << 53)..=(1 << 53)).contains(&value) {
            return Err(CvarError::Domain(
                "VM cvar integer write requires a safe integer".to_string(),
            ));
        }
        self.integer = value as i32;
        Ok(())
    }
}

enum MirrorDirective {
    MirrorChanged { name: String, value: String },
    RefreshFromOwner,
}

struct MirrorShared {
    queue: VecDeque<MirrorDirective>,
    refreshing: bool,
    assert_current: Rc<dyn Fn()>,
}

struct OwnerSubscription {
    clients: Vec<Weak<RefCell<MirrorShared>>>,
    token: BindingToken,
    owner: Weak<RefCell<CvarRegistry>>,
}

thread_local! {
    static MIRROR_SUBSCRIPTIONS: RefCell<HashMap<(u64, String), OwnerSubscription>> =
        RefCell::new(HashMap::new());
}

fn with_mirror_subscriptions<T>(access: impl FnOnce(&mut HashMap<(u64, String), OwnerSubscription>) -> T) -> T {
    MIRROR_SUBSCRIPTIONS.with(|subscriptions| access(&mut subscriptions.borrow_mut()))
}

/// Selected engine controls share values while each client retains its own
/// unrelated settings (donor `SharedCvarMirror`).
///
/// Rust adaptation: the donor synchronizes registries synchronously inside
/// `changed` callbacks, which would re-borrow a `RefCell` registry here.
/// Bindings therefore enqueue directives and [`SharedCvarMirror::pump`]
/// applies them once the triggering write has released its borrow; every
/// donor state transition (refresh, mirror-to-owner push, multi-client
/// fan-out) is preserved, only the scheduling is explicit.
pub struct SharedCvarMirror {
    owner: Rc<RefCell<CvarRegistry>>,
    mirror: Rc<RefCell<CvarRegistry>>,
    names: Vec<String>,
    shared: Rc<RefCell<MirrorShared>>,
    mirror_tokens: Vec<BindingToken>,
    closed: bool,
}

impl SharedCvarMirror {
    /// Attach a mirror: registers the owner's defaults in the mirror,
    /// refreshes values, and installs both directions of bindings.
    /// `assert_current` runs before every mirror-to-owner push.
    pub fn attach(
        owner: Rc<RefCell<CvarRegistry>>,
        mirror: Rc<RefCell<CvarRegistry>>,
        names: &[String],
        assert_current: Rc<dyn Fn()>,
    ) -> Result<Self, CvarError> {
        {
            let owner_ref = owner.borrow();
            let mirror_ref = mirror.borrow();
            if Rc::ptr_eq(&owner, &mirror)
                || owner_ref.session() != mirror_ref.session()
                || owner_ref.session().is_none()
                || owner_ref.dialect() != mirror_ref.dialect()
            {
                return Err(CvarError::Domain(
                    "Shared cvar mirror requires distinct registries in the same session and dialect".to_string(),
                ));
            }
        }
        for name in names {
            let owned = owner
                .borrow()
                .get(name)
                .ok_or_else(|| CvarError::Domain(format!("Shared engine cvar {name} is not declared")))?;
            mirror
                .borrow_mut()
                .register(name, &owned.reset_value.clone(), owned.flags)?;
        }
        let mut attached = Self {
            owner,
            mirror,
            names: names.to_vec(),
            shared: Rc::new(RefCell::new(MirrorShared {
                queue: VecDeque::new(),
                refreshing: false,
                assert_current,
            })),
            mirror_tokens: Vec::new(),
            closed: false,
        };
        attached.refresh()?;
        let installed = attached.install();
        if let Err(error) = installed {
            attached.close();
            return Err(error);
        }
        Ok(attached)
    }

    fn install(&mut self) -> Result<(), CvarError> {
        for name in self.names.clone() {
            let shared = Rc::downgrade(&self.shared);
            let changed_name = name.clone();
            let token = self.mirror.borrow_mut().bind_value(
                &name,
                Box::new(FnValueBinding::new(
                    |_| None,
                    move |value| {
                        if let Some(shared) = shared.upgrade() {
                            let mut shared = shared.borrow_mut();
                            if !shared.refreshing {
                                shared.queue.push_back(MirrorDirective::MirrorChanged {
                                    name: changed_name.clone(),
                                    value: value.to_string(),
                                });
                            }
                        }
                    },
                )),
            )?;
            self.mirror_tokens.push(token);
            self.subscribe_owner(&name)?;
        }
        Ok(())
    }

    fn subscribe_owner(&self, name: &str) -> Result<(), CvarError> {
        let owner_id = self.owner.borrow().registry_id();
        let key = self.owner.borrow().key(name);
        let client = Rc::downgrade(&self.shared);
        let joined = with_mirror_subscriptions(|subscriptions| {
            if let Some(subscription) = subscriptions.get_mut(&(owner_id, key.clone())) {
                subscription.clients.retain(|existing| existing.upgrade().is_some());
                subscription.clients.push(client);
                return true;
            }
            false
        });
        if joined {
            return Ok(());
        }
        let fanout_key = (owner_id, key.clone());
        let token = self.owner.borrow_mut().bind_value(
            name,
            Box::new(FnValueBinding::new(
                |_| None,
                move |_| {
                    with_mirror_subscriptions(|subscriptions| {
                        if let Some(subscription) = subscriptions.get_mut(&fanout_key) {
                            subscription.clients.retain(|existing| existing.upgrade().is_some());
                            for existing in &subscription.clients {
                                if let Some(shared) = existing.upgrade() {
                                    shared.borrow_mut().queue.push_back(MirrorDirective::RefreshFromOwner);
                                }
                            }
                        }
                    });
                },
            )),
        )?;
        with_mirror_subscriptions(|subscriptions| {
            subscriptions.insert(
                (owner_id, key),
                OwnerSubscription {
                    clients: vec![Rc::downgrade(&self.shared)],
                    token,
                    owner: Rc::downgrade(&self.owner),
                },
            );
        });
        Ok(())
    }

    /// Copy every mirrored value from the owner into the mirror.
    pub fn refresh(&self) -> Result<(), CvarError> {
        self.shared.borrow_mut().refreshing = true;
        let result = self.refresh_inner();
        self.shared.borrow_mut().refreshing = false;
        result
    }

    fn refresh_inner(&self) -> Result<(), CvarError> {
        for name in self.names.clone() {
            let text = self.owner.borrow().variable_string(&name);
            self.mirror.borrow_mut().set(&name, &text, true)?;
        }
        Ok(())
    }

    /// Apply queued cross-registry writes until the queue drains.
    pub fn pump(&self) -> Result<(), CvarError> {
        loop {
            let directive = self.shared.borrow_mut().queue.pop_front();
            match directive {
                None => return Ok(()),
                Some(MirrorDirective::RefreshFromOwner) => self.refresh()?,
                Some(MirrorDirective::MirrorChanged { name, value }) => {
                    let assert_current = self.shared.borrow().assert_current.clone();
                    assert_current();
                    self.owner.borrow_mut().set(&name, &value, true)?;
                }
            }
        }
    }

    /// Release every binding installed by this mirror.
    pub fn close(&mut self) {
        for token in self.mirror_tokens.drain(..) {
            self.mirror.borrow_mut().release_value_binding(token);
        }
        let owner_id = self.owner.borrow().registry_id();
        let keys: Vec<String> = self.names.iter().map(|name| self.owner.borrow().key(name)).collect();
        let shared = Rc::clone(&self.shared);
        let releases: Vec<(Weak<RefCell<CvarRegistry>>, BindingToken)> = with_mirror_subscriptions(|subscriptions| {
            let mut releases = Vec::new();
            for key in keys {
                let remove = match subscriptions.get_mut(&(owner_id, key.clone())) {
                    None => false,
                    Some(subscription) => {
                        subscription
                            .clients
                            .retain(|client| client.upgrade().is_some_and(|state| !Rc::ptr_eq(&state, &shared)));
                        if subscription.clients.is_empty() {
                            releases.push((subscription.owner.clone(), subscription.token));
                            true
                        } else {
                            false
                        }
                    }
                };
                if remove {
                    subscriptions.remove(&(owner_id, key));
                }
            }
            releases
        });
        for (owner, token) in releases {
            if let Some(owner) = owner.upgrade() {
                owner.borrow_mut().release_value_binding(token);
            }
        }
        self.closed = true;
    }

    /// Whether the mirror has been closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

#[cfg(test)]
mod tests {
    use super::flags;
    use super::*;

    #[test]
    fn quake_atof_matches_donor() {
        assert_eq!(quake_atof("-42"), -42.0);
        assert_eq!(quake_atof("0x10"), 16.0);
        assert_eq!(quake_atof("0Xff"), 255.0);
        assert_eq!(quake_atof("'A"), 65.0);
        assert_eq!(quake_atof("3.5"), 3.5);
        assert_eq!(quake_atof("12abc"), 12.0);
        assert_eq!(quake_atof(""), 0.0);
    }

    #[test]
    fn cvar_value_text_rounds_like_donor() {
        assert_eq!(cvar_value_text(5.0, true).unwrap(), "5");
        assert_eq!(cvar_value_text(5.0, false).unwrap(), "5.000000");
        assert_eq!(cvar_value_text(0.007_812_5, false).unwrap(), "0.007812");
        assert_eq!(cvar_value_text(0.023_437_5, false).unwrap(), "0.023438");
        assert_eq!(cvar_value_text(-0.0, true).unwrap(), "0");
        assert!(cvar_value_text(f64::INFINITY, false).is_err());
    }

    #[test]
    fn info_strings_follow_family_rules() {
        let mut printed = Vec::new();
        let options = InfoOptions {
            dialect: Dialect::Q3,
            maximum_length: 1024,
            target: InfoTarget::ServerInfo,
            server_high_characters: false,
        };
        let info = set_info_value("", "map", "q3dm1", options, &mut |text| printed.push(text.to_string())).unwrap();
        assert_eq!(info, "\\map\\q3dm1");
        let removed = set_info_value(&info, "map", "", options, &mut |text| printed.push(text.to_string())).unwrap();
        assert_eq!(removed, "");
        let kept = set_info_value("", "a\\b", "c", options, &mut |text| printed.push(text.to_string())).unwrap();
        assert_eq!(kept, "");
        assert!(!printed.is_empty());
    }

    #[test]
    fn registry_tracks_values_newest_first() {
        let mut registry = CvarRegistry::new(Dialect::Q3);
        registry.register("sv_fps", "20", flags::ARCHIVE).unwrap();
        registry.register("g_speed", "320", flags::NONE).unwrap();
        let names: Vec<String> = registry
            .snapshots(0)
            .into_iter()
            .map(|snapshot| snapshot.name)
            .collect();
        assert_eq!(names, vec!["g_speed".to_string(), "sv_fps".to_string()]);
        let updated = registry.set("SV_FPS", "30", false).unwrap().unwrap();
        assert_eq!(updated.value, "30");
        assert_eq!(updated.integer_value, 30);
        assert!((f64::from(updated.numeric_value) - 30.0).abs() < f64::EPSILON);
        assert_eq!(registry.variable_string("sv_fps"), "30");
        let missing = CvarRegistry::new(Dialect::Q1Netquake).variable_string("nope");
        assert_eq!(missing, "");
    }

    #[test]
    fn latch_restart_and_cheat_rules_match_donor() {
        let mut registry = CvarRegistry::new(Dialect::Q3);
        registry.register("r_mode", "3", flags::LATCH).unwrap();
        let latched = registry.set("r_mode", "4", false).unwrap().unwrap();
        assert_eq!(latched.value, "3");
        assert_eq!(latched.latched_value.as_deref(), Some("4"));
        let applied = registry.apply_latched(None).unwrap();
        assert_eq!(applied[0].value, "4");

        registry.register("god", "0", flags::CHEAT).unwrap();
        let denied = registry.set("god", "1", false).unwrap().unwrap();
        assert_eq!(denied.value, "1");
        registry.register("sv_cheats", "0", flags::NONE).unwrap();
        let blocked = registry.set("god", "0", false).unwrap().unwrap();
        assert_eq!(blocked.value, "1");

        let mut q2 = CvarRegistry::new(Dialect::Q2Classic);
        q2.register("gl_mode", "3", q2_flags::LATCH).unwrap();
        q2.set_server_active(true);
        let held = q2.set("gl_mode", "4", false).unwrap().unwrap();
        assert_eq!(held.latched_value.as_deref(), Some("4"));
        let applied = q2.apply_latched(None).unwrap();
        assert_eq!(applied[0].value, "4");

        let mut q1 = CvarRegistry::new(Dialect::Q1Netquake);
        assert!(q1.set("nope", "1", false).unwrap().is_none());
        q1.register("temp", "1", flags::NONE).unwrap();
        q1.set_command_flags("temp2", "2", SetCommandKind::Archive).unwrap();
        assert!(q1.is_console_created("temp2"));
        let commands = q1.archive_commands(&|_| true);
        assert!(commands.iter().any(|line| line == "seta temp2 \"2\""));
    }

    fn documented(summary: &str) -> CvarDocumentation {
        CvarDocumentation {
            summary: summary.to_string(),
            usage: "usage".to_string(),
            examples: Vec::new(),
            allowed_values: None,
        }
    }

    #[test]
    fn aliases_project_and_write_through() {
        let mut registry = CvarRegistry::new(Dialect::Q3);
        registry.register("sensitivity", "5", flags::ARCHIVE).unwrap();
        registry
            .register_alias(CvarAlias {
                name: "sens".to_string(),
                target: "sensitivity".to_string(),
                documentation: documented("mouse"),
                conversion: CvarAliasConversion::Identity,
            })
            .unwrap();
        assert_eq!(registry.canonical_name("sens"), "sensitivity");
        assert_eq!(registry.get("sens").unwrap().value, "5");
        registry.set("sens", "7", false).unwrap();
        assert_eq!(registry.variable_string("sensitivity"), "7");
        let names: Vec<String> = registry
            .snapshots(0)
            .into_iter()
            .map(|snapshot| snapshot.name)
            .collect();
        assert!(names.contains(&"sensitivity".to_string()));
        assert!(names.contains(&"sens".to_string()));
    }

    #[test]
    fn converted_aliases_translate_both_directions() {
        let mut registry = CvarRegistry::new(Dialect::Q3);
        registry.register("r_mode", "3", flags::NONE).unwrap();
        registry
            .register_alias(CvarAlias {
                name: "vid_mode".to_string(),
                target: "r_mode".to_string(),
                documentation: documented("video"),
                conversion: CvarAliasConversion::Converted {
                    read: Box::new(|value| format!("mode-{value}")),
                    write: Box::new(|value| {
                        value
                            .strip_prefix("mode-")
                            .map(str::to_string)
                            .ok_or_else(|| "expected mode-<n>".to_string())
                    }),
                },
            })
            .unwrap();
        assert_eq!(registry.get("vid_mode").unwrap().value, "mode-3");
        registry.set("vid_mode", "mode-4", false).unwrap();
        assert_eq!(registry.variable_string("r_mode"), "4");
        assert!(registry.set("vid_mode", "bogus", false).unwrap().is_none());
        assert!(!registry.take_notifications().is_empty());
    }

    #[test]
    fn alias_registration_rejects_bad_targets() {
        let mut registry = CvarRegistry::new(Dialect::Q3);
        registry.register("base", "1", flags::NONE).unwrap();
        let missing = registry.register_alias(CvarAlias {
            name: "alias".to_string(),
            target: "nope".to_string(),
            documentation: documented("x"),
            conversion: CvarAliasConversion::Identity,
        });
        assert!(missing.is_err());
        registry
            .register_alias(CvarAlias {
                name: "alias".to_string(),
                target: "base".to_string(),
                documentation: documented("x"),
                conversion: CvarAliasConversion::Identity,
            })
            .unwrap();
        let again = registry.register_alias(CvarAlias {
            name: "alias".to_string(),
            target: "base".to_string(),
            documentation: documented("x"),
            conversion: CvarAliasConversion::Identity,
        });
        assert!(again.is_err());
        let chained = registry.register_alias(CvarAlias {
            name: "chain".to_string(),
            target: "alias".to_string(),
            documentation: documented("x"),
            conversion: CvarAliasConversion::Identity,
        });
        assert!(chained.is_err());
        // Declaring an alias name registers nothing and keeps canonical policy.
        let declared = registry.register("alias", "9", flags::NONE).unwrap().unwrap();
        assert_eq!(declared.value, "1");
        assert_eq!(registry.variable_string("base"), "1");
    }

    #[test]
    fn alias_info_flags_need_explicit_mapping() {
        let mut registry = CvarRegistry::new(Dialect::Q3);
        registry.register("name", "p", flags::USER_INFO).unwrap();
        let aliased = registry.register_alias(CvarAlias {
            name: "player_name".to_string(),
            target: "name".to_string(),
            documentation: documented("x"),
            conversion: CvarAliasConversion::Identity,
        });
        assert!(aliased.is_err());
        registry.register("plain", "1", flags::NONE).unwrap();
        registry
            .register_alias(CvarAlias {
                name: "plain_alias".to_string(),
                target: "plain".to_string(),
                documentation: documented("x"),
                conversion: CvarAliasConversion::Identity,
            })
            .unwrap();
        assert!(registry.add_flags("plain_alias", flags::USER_INFO).is_err());
        assert!(registry.add_flags("plain_alias", flags::ARCHIVE).is_ok());
        assert_eq!(registry.get("plain").unwrap().flags & flags::ARCHIVE, flags::ARCHIVE);
    }

    #[test]
    fn value_bindings_validate_and_observe() {
        let mut registry = CvarRegistry::new(Dialect::Q3);
        registry.register("sv_fps", "20", flags::NONE).unwrap();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let capture = Rc::clone(&seen);
        let token = registry
            .bind_value(
                "sv_fps",
                Box::new(FnValueBinding::new(
                    |value| {
                        if value.parse::<i32>().is_ok() {
                            None
                        } else {
                            Some("expected an integer".to_string())
                        }
                    },
                    move |value| capture.borrow_mut().push(value.to_string()),
                )),
            )
            .unwrap();
        assert!(registry.set("sv_fps", "fast", false).unwrap().is_none());
        assert!(!registry.take_notifications().is_empty());
        registry.set("sv_fps", "30", false).unwrap();
        // Equal writes still notify bindings.
        registry.set("sv_fps", "30", true).unwrap();
        assert_eq!(*seen.borrow(), vec!["30".to_string(), "30".to_string()]);
        assert!(registry.release_value_binding(token));
        assert!(!registry.release_value_binding(token));
        registry.set("sv_fps", "40", false).unwrap();
        assert_eq!(seen.borrow().len(), 2);
    }

    #[test]
    fn binding_rules_match_donor() {
        let mut registry = CvarRegistry::new(Dialect::Q3);
        assert!(registry
            .bind_value("nope", Box::new(FnValueBinding::new(|_| None, |_| {})))
            .is_err());
        registry.register("v", "1", flags::NONE).unwrap();
        registry
            .bind_value("v", Box::new(FnValueBinding::new(|_| None, |_| {})))
            .unwrap();
        let duplicate = registry.bind_value("v", Box::new(FnValueBinding::new(|_| None, |_| {})));
        assert!(duplicate.is_err());
        registry
            .register_alias(CvarAlias {
                name: "w".to_string(),
                target: "v".to_string(),
                documentation: documented("x"),
                conversion: CvarAliasConversion::Identity,
            })
            .unwrap();
        let via_alias = registry.bind_value("w", Box::new(FnValueBinding::new(|_| None, |_| {})));
        assert!(via_alias.is_err());
    }

    #[test]
    fn documents_fall_back_to_alias_help() {
        let mut registry = CvarRegistry::new(Dialect::Q3);
        registry.register("v", "1", flags::NONE).unwrap();
        assert!(registry.documentation("v").is_none());
        registry.document("v", documented("direct")).unwrap();
        assert_eq!(registry.documentation("v").unwrap().summary, "direct");
        registry
            .register_alias(CvarAlias {
                name: "w".to_string(),
                target: "v".to_string(),
                documentation: documented("alias-help"),
                conversion: CvarAliasConversion::Identity,
            })
            .unwrap();
        assert_eq!(registry.documentation("w").unwrap().summary, "alias-help");
        assert!(registry.document("nope", documented("x")).is_err());
    }

    #[test]
    fn vm_mirrors_track_the_counter() {
        let registry = Rc::new(RefCell::new(CvarRegistry::new(Dialect::Q3)));
        let mut vm = RegistryVmCvar::registered(Rc::clone(&registry), "g_speed", "320", flags::NONE).unwrap();
        assert_eq!(vm.value(), "320");
        assert_eq!(vm.integer_value(), 320);
        assert_eq!(vm.modification_count(), 1);
        registry.borrow_mut().set("g_speed", "400", true).unwrap();
        assert_eq!(vm.value(), "320");
        vm.update().unwrap();
        assert_eq!(vm.value(), "400");
        assert_eq!(vm.modification_count(), 2);
        vm.write_integer(7).unwrap();
        assert_eq!(vm.integer_value(), 7);
        assert_eq!(registry.borrow().variable_string("g_speed"), "400");
        assert!(vm.write_integer(1 << 60).is_err());
        assert!(registry.borrow().read_vm(999).is_err());
    }

    #[test]
    fn converted_aliases_get_vm_handles() {
        let mut registry = CvarRegistry::new(Dialect::Q3);
        registry.register("r_mode", "3", flags::NONE).unwrap();
        registry
            .register_alias(CvarAlias {
                name: "vid_mode".to_string(),
                target: "r_mode".to_string(),
                documentation: documented("video"),
                conversion: CvarAliasConversion::Converted {
                    read: Box::new(|value| format!("mode-{value}")),
                    write: Box::new(|value| Ok(value.to_string())),
                },
            })
            .unwrap();
        let handle = registry.bind_vm("vid_mode", "mode-3", flags::NONE).unwrap();
        assert_eq!(registry.bind_vm("vid_mode", "mode-3", flags::NONE).unwrap(), handle);
        assert_eq!(registry.read_vm(handle).unwrap().unwrap().value, "mode-3");
        assert!(registry.index_count() > 1);
    }

    #[test]
    fn save_round_trips_through_prepare() {
        let mut registry = CvarRegistry::new(Dialect::Q3);
        registry.register("a", "1", flags::ARCHIVE).unwrap();
        registry.register("b", "2", flags::LATCH).unwrap();
        registry.set("b", "3", false).unwrap();
        let image = registry.capture_save_state().unwrap();
        let mut restored = CvarRegistry::new(Dialect::Q3);
        let pending = restored.prepare_registry_restore(&image).unwrap();
        pending.apply(&mut restored, Vec::new());
        assert_eq!(restored.capture_save_state().unwrap(), image);
        assert_eq!(restored.get("b").unwrap().latched_value.as_deref(), Some("3"));
    }

    #[test]
    fn save_restore_validates_images() {
        let mut registry = CvarRegistry::new(Dialect::Q1Netquake);
        registry.register("s", "1", flags::SERVER_INFO).unwrap();
        registry.set_server_active(true);
        registry.set("s", "2", false).unwrap();
        assert!(registry.capture_save_state().is_err());
        let transfer = registry.capture_world_transfer_state();
        let _ = registry.take_effects();
        let image = registry.capture_save_state().unwrap();
        assert_eq!(image.variables.len(), transfer.variables.len());

        let mut q3 = CvarRegistry::new(Dialect::Q3);
        assert!(q3.restore_save_state(&image).is_err());

        let mut bad_order = image.clone();
        bad_order.order.push("extra".to_string());
        let mut fresh = CvarRegistry::new(Dialect::Q1Netquake);
        assert!(fresh.restore_save_state(&bad_order).is_err());

        let mut missing_bound = CvarRegistry::new(Dialect::Q1Netquake);
        missing_bound.register("s", "1", flags::NONE).unwrap();
        missing_bound
            .bind_value("s", Box::new(FnValueBinding::new(|_| None, |_| {})))
            .unwrap();
        let mut dropped = image.clone();
        dropped.variables.clear();
        dropped.order.clear();
        assert!(missing_bound.restore_save_state(&dropped).is_err());

        let mut qc = CvarRegistry::new(Dialect::Q1Netquake);
        let bed = qc.capture_quake_c_state().unwrap();
        qc.restore_quake_c_state(&bed).unwrap();
        assert!(CvarRegistry::new(Dialect::Q3).capture_quake_c_state().is_err());
    }

    #[test]
    fn archive_round_trips() {
        let mut registry = CvarRegistry::new(Dialect::Q3);
        registry.register("a", "1", flags::ARCHIVE).unwrap();
        let entries = registry.archive_entries(&|_| true);
        let mut fresh = CvarRegistry::new(Dialect::Q3);
        fresh.apply_archive(&entries).unwrap();
        assert_eq!(fresh.variable_string("a"), "1");
    }

    #[test]
    fn effects_carry_the_registry_session() {
        use crate::identity::IdentityOwner;
        let owner = IdentityOwner::create("effect-session").unwrap();
        let mut registry = CvarRegistry::with_session(Dialect::Q1Netquake, Some(owner.session().clone()));
        registry.register("hostname", "a", flags::SERVER_INFO).unwrap();
        registry.set_server_active(true);
        registry.set("hostname", "b", false).unwrap();
        let effects = registry.take_effects();
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            CvarEffect::Broadcast { session, text } => {
                assert_eq!(*session, Some(owner.session().clone()));
                assert!(text.contains("hostname"));
            }
            other => panic!("unexpected effect: {other:?}"),
        }
    }

    #[test]
    fn shared_mirror_syncs_both_directions() {
        use crate::identity::IdentityOwner;
        let owner_id = IdentityOwner::create("mirror-session").unwrap();
        let session = owner_id.session().clone();
        let owner = Rc::new(RefCell::new(CvarRegistry::with_session(
            Dialect::Q3,
            Some(session.clone()),
        )));
        let mirror = Rc::new(RefCell::new(CvarRegistry::with_session(Dialect::Q3, Some(session))));
        owner.borrow_mut().register("timescale", "1", flags::CHEAT).unwrap();
        owner.borrow_mut().register("local_only", "x", flags::NONE).unwrap();
        let mut attached = SharedCvarMirror::attach(
            Rc::clone(&owner),
            Rc::clone(&mirror),
            &["timescale".to_string()],
            Rc::new(|| {}),
        )
        .unwrap();
        assert_eq!(mirror.borrow().variable_string("timescale"), "1");
        assert!(mirror.borrow().get("local_only").is_none());

        owner.borrow_mut().set("timescale", "2", true).unwrap();
        attached.pump().unwrap();
        assert_eq!(mirror.borrow().variable_string("timescale"), "2");

        mirror.borrow_mut().set("timescale", "3", true).unwrap();
        attached.pump().unwrap();
        assert_eq!(owner.borrow().variable_string("timescale"), "3");
        // The owner write fans back out; the mirror converges without loops.
        attached.pump().unwrap();
        assert_eq!(mirror.borrow().variable_string("timescale"), "3");

        attached.close();
        assert!(attached.is_closed());
        owner.borrow_mut().set("timescale", "4", true).unwrap();
        attached.pump().unwrap();
        assert_eq!(mirror.borrow().variable_string("timescale"), "3");
    }

    #[test]
    fn shared_mirror_validates_and_shares_subscriptions() {
        use crate::identity::IdentityOwner;
        let owner_id = IdentityOwner::create("mirror-share").unwrap();
        let session = owner_id.session().clone();
        let owner = Rc::new(RefCell::new(CvarRegistry::with_session(
            Dialect::Q3,
            Some(session.clone()),
        )));
        let first = Rc::new(RefCell::new(CvarRegistry::with_session(
            Dialect::Q3,
            Some(session.clone()),
        )));
        let second = Rc::new(RefCell::new(CvarRegistry::with_session(Dialect::Q3, Some(session))));
        owner.borrow_mut().register("v", "1", flags::NONE).unwrap();
        let mut a =
            SharedCvarMirror::attach(Rc::clone(&owner), Rc::clone(&first), &["v".to_string()], Rc::new(|| {})).unwrap();
        let mut b = SharedCvarMirror::attach(
            Rc::clone(&owner),
            Rc::clone(&second),
            &["v".to_string()],
            Rc::new(|| {}),
        )
        .unwrap();
        owner.borrow_mut().set("v", "2", true).unwrap();
        a.pump().unwrap();
        b.pump().unwrap();
        assert_eq!(first.borrow().variable_string("v"), "2");
        assert_eq!(second.borrow().variable_string("v"), "2");
        // Closing one client keeps the shared owner subscription alive.
        a.close();
        owner.borrow_mut().set("v", "3", true).unwrap();
        b.pump().unwrap();
        assert_eq!(second.borrow().variable_string("v"), "3");
        b.close();

        let other_session = IdentityOwner::create("other").unwrap().session().clone();
        let foreign = Rc::new(RefCell::new(CvarRegistry::with_session(
            Dialect::Q3,
            Some(other_session),
        )));
        assert!(SharedCvarMirror::attach(
            Rc::clone(&owner),
            Rc::clone(&foreign),
            &["v".to_string()],
            Rc::new(|| {})
        )
        .is_err());
        assert!(
            SharedCvarMirror::attach(Rc::clone(&owner), Rc::clone(&owner), &["v".to_string()], Rc::new(|| {})).is_err()
        );
        let missing = Rc::new(RefCell::new(CvarRegistry::with_session(
            Dialect::Q3,
            Some(owner_id.session().clone()),
        )));
        assert!(SharedCvarMirror::attach(
            Rc::clone(&owner),
            Rc::clone(&missing),
            &["nope".to_string()],
            Rc::new(|| {})
        )
        .is_err());
    }
}
